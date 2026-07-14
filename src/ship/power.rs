use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{
    CoordinationPayload, InterSystemPayload, InterSystemQueue, PowerBatteryBlackboard,
    PowerBlackboard, PowerGroupEntry, PowerGroupId, PowerReactorBlackboard, ServerMessage,
    StationId, SystemBlackboard, SystemId,
};
use crate::modifiers::power_system::{
    power_level_for_group, PowerConfig, PowerSystem, HELM_POWER_GROUP, POWER_GROUP_ORDER,
    SENSORS_POWER_GROUP, WEAPONS_POWER_GROUP,
};
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::{CoordinationEnqueue, LastHelmInput};

// ── Resources ──────────────────────────────────────────────────────────────────

/// Wraps the pure-Rust power system so it can be used as a Bevy resource.
///
/// Derives both `Resource` (existing player-ship singleton) and `Component`
/// (per-entity path after issue #594 unification).
#[derive(Resource, Component, Clone)]
pub struct ShipPowerSystem(pub PowerSystem);

/// Wraps the power config for the ship's power system.
///
/// Dual-derives `Resource` (legacy global fallback used by tests) and
/// `Component` (per-entity storage on each ship — PR 6 migration, see PRD #597).
#[derive(Resource, Component, Default, Clone)]
pub struct PowerConfigResource(pub PowerConfig);

/// Per-group power multiplier configuration: `[f32; 4]` indexed by level-1
/// (index 0 = level 1, index 3 = level 4). Defaults give `[-0.5, 0.0, 0.25, 0.5]`
/// for every canonical power group unless overridden in the ship TOML.
///
/// After issue #617 the map is keyed by [`PowerGroupId`] rather than `Console`.
///
/// Dual-derives `Resource` (legacy global fallback used by tests) and
/// `Component` (per-entity storage on each ship — PR 6 migration, see PRD #597).
#[derive(Resource, Component, Clone, Debug)]
pub struct PowerMultiplierResource {
    pub multipliers: std::collections::HashMap<PowerGroupId, [f32; 4]>,
}

impl Default for PowerMultiplierResource {
    fn default() -> Self {
        let defaults = [-0.5, 0.0, 0.25, 0.5];
        let mut multipliers = std::collections::HashMap::new();
        for &name in POWER_GROUP_ORDER {
            multipliers.insert(PowerGroupId(name.to_string()), defaults);
        }
        Self { multipliers }
    }
}

// ── AI config ─────────────────────────────────────────────────────────────────

/// TOML-loaded configuration for the power AI controller.
///
/// Loaded from `[power.ai]` in the ship entity TOML and inserted as a resource
/// at startup by the entity spawner. The fields mirror the `[power.ai]` TOML
/// keys; defaults are used when the section is absent.
///
/// Dual-derives `Resource` (legacy global fallback used by tests) and
/// `Component` (per-entity storage on each ship — PR 6 migration, see PRD #597).
#[derive(Resource, Component, Clone, Debug)]
pub struct PowerAiConfigResource {
    /// Minimum battery charge fraction (0.0–1.0) before the AI boosts weapons power.
    pub weapons_battery_floor: f32,
    /// Minimum battery charge fraction (0.0–1.0) before the AI boosts shields power.
    /// NOTE: PowerSystem has no dedicated shields field; this is reserved for future use.
    pub shields_battery_floor: f32,
    /// Minimum battery charge fraction (0.0–1.0) before the AI boosts helm power.
    pub helm_battery_floor: f32,
    /// Thrust level (0.0–1.0) above which the AI considers the ship actively moving.
    pub helm_throttle_threshold: f32,
}

impl Default for PowerAiConfigResource {
    fn default() -> Self {
        Self {
            weapons_battery_floor: 0.5,
            shields_battery_floor: 0.25,
            helm_battery_floor: 0.75,
            helm_throttle_threshold: 0.5,
        }
    }
}

/// Debounce state for power brownout coordination advisories (issue #678).
///
/// Tracks which power groups have already been notified of a brownout
/// condition so the advisory only fires once per transition into brownout.
/// Cleared when the group exits the brownout condition, allowing re-fire
/// on subsequent brownout cycles.
#[derive(Component, Default, Clone)]
pub struct PowerBrownoutState {
    /// Group id strings (e.g. "weapons", "helm", "sensors") that are
    /// currently in a notified-brownout state.
    pub notified_groups: std::collections::HashSet<String>,
}

/// Maps a canonical power group id string to its display label in the HTML
/// power panel. Anything unknown falls back to the upper-cased id string
/// (via the caller).
pub fn power_group_label(group_id: &str) -> &'static str {
    match group_id {
        HELM_POWER_GROUP => "HELM",
        WEAPONS_POWER_GROUP => "WEAPONS",
        SENSORS_POWER_GROUP => "SENSORS",
        _ => "UNKNOWN",
    }
}

/// Returns the current power level for `group` from the `PowerSystem`.
///
/// Delegates to [`power_level_for_group`]; kept as a thin wrapper for the
/// legacy call-site shape.
pub fn power_level_for(ps: &PowerSystem, group: &PowerGroupId) -> u8 {
    power_level_for_group(ps, group)
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipPowerPlugin;

impl Plugin for ShipPowerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::messages::InterSystemQueue>()
            .add_message::<CoordinationEnqueue>();
        app.insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<PowerAiConfigResource>()
            .add_systems(
                Update,
                (
                    handle_power_messages.in_set(crate::sim_sets::SimSet::Input),
                    tick_power_system.in_set(crate::sim_sets::SimSet::Physics),
                    operate_power_ai.in_set(crate::sim_sets::SimSet::Physics),
                    handle_power_inter_system.in_set(crate::sim_sets::SimSet::Modifiers),
                    tick_power_brownout_advisory.in_set(crate::sim_sets::SimSet::Modifiers),
                    publish_power_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            )
            .add_plugins(power_state_broadcaster());
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `PowerState` producer.
///
/// Broadcasts `PowerState` at 10 Hz to the `Power` console holder only.
/// This is the canonical registration; it is added by [`ShipPowerPlugin`]
/// and also by the test harness in `test_app()`.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipPowerSystem` component
/// on the LocalShip entity, falling back to the global `ShipPowerSystem`
/// resource for test harnesses that only insert the Resource form.
pub fn power_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::HoldingSystem(SystemId("power-reactor".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            // Prefer per-entity component on the LocalShip; fall back to the
            // global Resource for tests that only initialise the Resource.
            let mut q =
                world.query_filtered::<&ShipPowerSystem, With<crate::server_app::LocalShip>>();
            let power_snapshot = q
                .iter(world)
                .next()
                .cloned()
                .or_else(|| world.get_resource::<ShipPowerSystem>().cloned());
            let Some(power) = power_snapshot else {
                return vec![];
            };
            vec![ServerMessage::PowerState {
                helm: power.0.level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
                weapons: power.0.level_for(&PowerGroupId(WEAPONS_POWER_GROUP.into())),
                sensors: power.0.level_for(&PowerGroupId(SENSORS_POWER_GROUP.into())),
                battery_charge: power.0.battery_charge,
                locked: power.0.locked,
            }]
        },
    )
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetPowerGroupAllocation` messages from the Power console.
///
/// Validates: sender holds the `power` station. Reads `ControlSystem` messages
/// targeting the **reactor** fine system (`POWER_REACTOR_SYSTEM_ID`) with a
/// `SetPowerGroupAllocation` payload, and calls `PowerSystem::increase` / `decrease`
/// to reach the requested level. Per issue #513 the reactor OWNS the allocation
/// surface — a Disabled/Destroyed reactor's `accept_human_input` policy
/// (populated by `sync_console_damage_tiers`) refuses these messages at
/// admission, so the coarse `power` id no longer receives allocation input.
///
/// After PR 6 (PRD #597): mutates the per-entity `ShipPowerSystem` component
/// on the LocalShip entity when present, otherwise the global `ShipPowerSystem`
/// resource. When both are present, dual-writes so legacy readers stay in sync.
pub fn handle_power_messages(
    mut ship_query: Query<
        (
            &crate::messages::AdmittedCommands,
            Option<&mut ShipPowerSystem>,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    power_res: Option<ResMut<ShipPowerSystem>>,
) {
    let mut power_res = power_res;
    // Iterate every ship (player + NPC) so both the player's Power console
    // commands and the future NPC `operate_power_ai` writes into
    // `AdmittedCommands` re-allocate that ship's own power grid.
    for (admitted, mut power_comp, is_local) in ship_query.iter_mut() {
        let mut pending: Vec<(crate::messages::PowerGroupId, u8)> = Vec::new();
        for cmd in admitted.for_target(crate::system_registry::POWER_REACTOR_SYSTEM_ID) {
            if let crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, level } =
                &cmd.payload
            {
                pending.push((group.clone(), *level));
            }
        }
        if pending.is_empty() {
            continue;
        }
        for (group, level) in pending {
            if let Some(pc) = power_comp.as_deref_mut() {
                if let Err(err) = pc.0.set_group_allocation(&group, level) {
                    warn!("[power] ignored power allocation: {err:?}");
                }
            } else if is_local {
                if let Some(pr) = power_res.as_deref_mut() {
                    if let Err(err) = pr.0.set_group_allocation(&group, level) {
                        warn!("[power] ignored power allocation: {err:?}");
                    }
                }
            }
        }
        // Dual-write: keep the Resource in sync with the LocalShip's
        // Component when both exist (legacy Resource path for tests).
        if is_local {
            if let (Some(pc), Some(pr)) = (power_comp.as_deref(), power_res.as_deref_mut()) {
                pr.0 = pc.0.clone();
            }
        }
    }
}

/// Apply inter-system commands (e.g. `DrainWeaponsBattery` from Weapons).
///
/// Invariant-gated: no control-state check. Runs in `SimSet::Modifiers`,
/// after physics ticks have emitted their inter-system messages.
///
/// Routes by `source_entity` so every ship's own inter-system messages
/// mutate that ship's own per-entity `ShipPowerSystem` component. Falls
/// back to the LocalShip's Component (or the global Resource for legacy
/// test paths) when `source_entity` is `None`.
///
/// **Issue #513 battery offline gate.** Drain messages target the
/// [`crate::system_registry::POWER_BATTERY_SYSTEM_ID`] fine system. If the
/// battery is in `ShipSystemControlSources.offline_systems` (i.e. hull
/// damage put it into Disabled/Destroyed tier), the drain is refused —
/// the reserve pool is treated as inaccessible so weapons draws cannot
/// consume from it. The gate applies uniformly whether the mutation
/// would land on a per-entity `ShipPowerSystem` component or on the
/// fallback `ShipPowerSystem` Resource — `ShipSystemControlSources` is
/// consulted on the same ship entity in both paths.
pub fn handle_power_inter_system(
    queue: Res<InterSystemQueue>,
    mut ship_q: Query<
        (
            Entity,
            &mut ShipPowerSystem,
            Option<&PowerConfigResource>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    cs_only_q: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    power_res: Option<ResMut<ShipPowerSystem>>,
    config_res: Option<Res<PowerConfigResource>>,
) {
    let mut power_res = power_res;
    let battery_id = crate::system_registry::power_battery_system_id();
    // Snapshot the LocalShip entity so `source_entity: None` (legacy path)
    // resolves to the player. Collect per-entity references once so we can
    // dispatch a mutable borrow per message inside the loop.
    let local_ship_entity: Option<Entity> = ship_q
        .iter()
        .find_map(|(e, _, _, _, is_local)| if is_local { Some(e) } else { None })
        .or_else(|| {
            cs_only_q
                .iter()
                .find_map(|(e, _, is_local)| if is_local { Some(e) } else { None })
        });
    // Pre-collect the set of ships whose battery is offline (via the
    // control-sources-only query). Used to gate both the per-entity path
    // and the Resource fallback path so a Disabled/Destroyed battery
    // refuses drains regardless of where the mutation would land.
    let battery_offline_ships: std::collections::HashSet<Entity> = cs_only_q
        .iter()
        .filter_map(|(e, cs, _)| {
            if cs.0.offline_systems.contains(&battery_id) {
                Some(e)
            } else {
                None
            }
        })
        .collect();
    let mut applied_local = false;

    for msg in queue.for_target(crate::system_registry::POWER_BATTERY_SYSTEM_ID) {
        let target_entity = msg.source_entity.or(local_ship_entity);
        match &msg.payload {
            InterSystemPayload::DrainWeaponsBattery { amount } => {
                // Offline gate applies uniformly (per-entity + Resource paths).
                if let Some(target) = target_entity {
                    if battery_offline_ships.contains(&target) {
                        continue;
                    }
                }
                if let Some(target) = target_entity {
                    if let Ok((_, mut power_comp, cfg_comp, _cs_comp, is_local)) =
                        ship_q.get_mut(target)
                    {
                        let cfg_default;
                        let config: &PowerConfigResource = match cfg_comp {
                            Some(c) => c,
                            None => match config_res.as_deref() {
                                Some(c) => c,
                                None => {
                                    cfg_default = PowerConfigResource::default();
                                    &cfg_default
                                }
                            },
                        };
                        power_comp.0.battery_charge =
                            (power_comp.0.battery_charge - amount).clamp(0.0, config.0.capacity);
                        if is_local {
                            applied_local = true;
                        }
                        continue;
                    }
                }
                // Resource-only fallback for tests without a Ship entity
                // (or without a ShipPowerSystem component on the ship).
                let cfg_default;
                let config: &PowerConfigResource = match config_res.as_deref() {
                    Some(c) => c,
                    None => {
                        cfg_default = PowerConfigResource::default();
                        &cfg_default
                    }
                };
                if let Some(pr) = power_res.as_deref_mut() {
                    pr.0.battery_charge =
                        (pr.0.battery_charge - amount).clamp(0.0, config.0.capacity);
                }
            }
            // JoystickState messages are produced by the Helm fine systems (issue #511)
            // and are not relevant to the Power system — ignore them.
            InterSystemPayload::JoystickState { .. } => {}
            // ClaimTorpedoRound messages are produced by the Torpedo Tube fine systems
            // (issue #512) and consumed by the magazine handler — ignore them here.
            InterSystemPayload::ClaimTorpedoRound { .. } => {}
        }
    }

    // Dual-write: keep the Resource in sync with the LocalShip's Component
    // when both exist (legacy Resource path for tests).
    if applied_local {
        if let Some(local) = local_ship_entity {
            if let Ok((_, pc, _, _, _)) = ship_q.get(local) {
                if let Some(pr) = power_res.as_deref_mut() {
                    pr.0 = pc.0.clone();
                }
            }
        }
    }
}

/// Tick the power system battery charge each frame.
///
/// After PR 6 (PRD #597): iterates ALL ship entities with a `ShipPowerSystem`
/// component so NPC ships tick their own power. Uses the per-entity
/// `PowerConfigResource` component when present, else the global Resource
/// fallback, so NPC ships without a `[power]` block still tick with defaults.
/// The Resource fallback path is retained for test environments that only
/// insert the resource without a ship entity.
pub fn tick_power_system(
    time: Res<Time>,
    mut power_res: Option<ResMut<ShipPowerSystem>>,
    config_res: Option<Res<PowerConfigResource>>,
    mut ships: Query<
        (&mut ShipPowerSystem, Option<&PowerConfigResource>),
        With<crate::server_app::Ship>,
    >,
) {
    let dt = time.delta_secs();
    let mut ticked_any = false;
    for (mut power, config_comp) in ships.iter_mut() {
        let cfg_default;
        let cfg: &PowerConfigResource = match config_comp {
            Some(c) => c,
            None => match config_res.as_deref() {
                Some(c) => c,
                None => {
                    cfg_default = PowerConfigResource::default();
                    &cfg_default
                }
            },
        };
        power.0.tick(dt, &cfg.0);
        ticked_any = true;
    }
    // Fallback: no ship entity with the component (test environments that only
    // insert the Resource form). Tick the Resource directly.
    if !ticked_any {
        if let (Some(power), Some(config)) = (power_res.as_deref_mut(), config_res.as_deref()) {
            power.0.tick(dt, &config.0);
        }
    }
}

/// AI controller for the power console.
///
/// Rules (all purely advisory — clamped by PowerSystem bounds):
/// - High throttle AND sufficient battery → set Helm to 3
/// - Zero throttle → set Helm to 1 (idle)
/// - Otherwise → set Helm to 2
/// - Red alert AND sufficient battery → set Weapons to 3
///
/// PowerSystem has no shields field; shields_battery_floor is reserved for
/// future extension but produces no action today.
///
/// After PR 6 (PRD #597): iterates ALL ship entities where the Power system is
/// `ControlSource::Ai`. Each ship reads its own `ShipRedAlert`,
/// `LastHelmInput`, and `PowerConfigResource`/`PowerAiConfigResource`
/// components (all default when absent). NPCs and the player ship follow the
/// same code path — the only differentiators are the per-station control
/// sources and the components on each entity.
///
/// The global `ShipPowerSystem` resource is still dual-written from the
/// LocalShip entity's component so the legacy Resource-based broadcasters
/// keep working during the migration.
pub fn operate_power_ai(
    mut power_res: Option<ResMut<ShipPowerSystem>>,
    ai_config_res: Option<Res<PowerAiConfigResource>>,
    config_res: Option<Res<PowerConfigResource>>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    ship_comp_query: Query<
        &crate::ship_plugin::ShipConfigComponent,
        With<crate::server_app::LocalShip>,
    >,
    mut ship_power_q: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut ShipPowerSystem,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&LastHelmInput>,
            Option<&PowerConfigResource>,
            Option<&PowerAiConfigResource>,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    // Yield to any human Power console holder on the player ship. NPC ships
    // have no human console holders, so they always run the AI branch when
    // their Power system is under AI control.
    let human_holds_player_power =
        if let (Some(sessions), Some(_ship_config)) = (&sessions, ship_comp_query.iter().next()) {
            sessions
                .0
                .holder_for_station(&StationId("power".into()))
                .is_some()
        } else {
            false
        };

    let ai_cfg_default;
    let ai_cfg_fallback: &PowerAiConfigResource = match ai_config_res.as_deref() {
        Some(c) => c,
        None => {
            ai_cfg_default = PowerAiConfigResource::default();
            &ai_cfg_default
        }
    };
    let cfg_default;
    let cfg_fallback: &PowerConfigResource = match config_res.as_deref() {
        Some(c) => c,
        None => {
            cfg_default = PowerConfigResource::default();
            &cfg_default
        }
    };

    let mut player_power: Option<crate::modifiers::power_system::PowerSystem> = None;
    for (
        _entity,
        control_sources,
        mut power_comp,
        red_alert_comp,
        last_helm_comp,
        cfg_comp,
        ai_cfg_comp,
        is_local,
    ) in ship_power_q.iter_mut()
    {
        // Skip the player ship when a human is at the Power console; NPCs
        // never have a human holder so this only ever skips the player ship.
        if is_local && human_holds_player_power {
            continue;
        }
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::power_reactor_system_id());
        if !policy.operate_ai {
            continue;
        }

        let cfg: &PowerConfigResource = cfg_comp.unwrap_or(cfg_fallback);
        let ai_cfg: &PowerAiConfigResource = ai_cfg_comp.unwrap_or(ai_cfg_fallback);
        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);
        let throttle = last_helm_comp.map(|l| l.thrust).unwrap_or(0.0);

        let battery_pct = power_comp.0.battery_charge / cfg.0.capacity;

        let helm_id = PowerGroupId(HELM_POWER_GROUP.into());
        let weapons_id = PowerGroupId(WEAPONS_POWER_GROUP.into());
        let sensors_id = PowerGroupId(SENSORS_POWER_GROUP.into());

        // Weapons: boost on red alert when battery allows.
        if red_alert && battery_pct >= ai_cfg.weapons_battery_floor {
            let _ = power_comp.0.set_group_allocation(&weapons_id, 3);
        }

        // Helm: scale with throttle demand and battery availability.
        if throttle > ai_cfg.helm_throttle_threshold && battery_pct >= ai_cfg.helm_battery_floor {
            let _ = power_comp.0.set_group_allocation(&helm_id, 3);
        } else if throttle == 0.0 {
            let _ = power_comp.0.set_group_allocation(&helm_id, 1);
            if !red_alert || battery_pct < ai_cfg.weapons_battery_floor {
                let _ = power_comp.0.set_group_allocation(&weapons_id, 2);
            }
        } else {
            let _ = power_comp.0.set_group_allocation(&helm_id, 2);
            if !red_alert || battery_pct < ai_cfg.weapons_battery_floor {
                let _ = power_comp.0.set_group_allocation(&weapons_id, 2);
            }
        }

        // Clamp all groups to [1, 4] — set_group_allocation already clamps
        // but preserve the explicit clamping call for legibility and to keep
        // the sensors group in-range if some future code path pokes it.
        let helm_level = power_comp.0.level_for(&helm_id).clamp(1, 4);
        let weapons_level = power_comp.0.level_for(&weapons_id).clamp(1, 4);
        let sensors_level = power_comp.0.level_for(&sensors_id).clamp(1, 4);
        let _ = power_comp.0.set_group_allocation(&helm_id, helm_level);
        let _ = power_comp
            .0
            .set_group_allocation(&weapons_id, weapons_level);
        let _ = power_comp
            .0
            .set_group_allocation(&sensors_id, sensors_level);

        if is_local {
            player_power = Some(power_comp.0.clone());
        }
    }

    // Sync the global resource with the player ship's component (dual-write).
    if let (Some(power_res), Some(player_power)) = (power_res.as_deref_mut(), player_power) {
        power_res.0 = player_power;
    }
}

// ── Power brownout advisory (issue #678) ─────────────────────────────────────

/// Map a power group id string to the target `SystemId` for coordination.
fn system_id_for_power_group(group: &str) -> Option<SystemId> {
    match group {
        WEAPONS_POWER_GROUP => Some(crate::system_registry::tactical_system_id()),
        HELM_POWER_GROUP => Some(crate::system_registry::helm_system_id()),
        SENSORS_POWER_GROUP => Some(crate::system_registry::sensors_system_id()),
        _ => None,
    }
}

/// Emit power brownout coordination advisories for groups with active demand
/// that cannot be satisfied (total allocation > 6 → battery draining).
///
/// An advisory fires **only** when:
/// - Total allocation > 6 (battery is draining — supply cannot meet demand)
/// - The group's allocation level > 1 (system is actively drawing, not idle)
///
/// Debounced via [`PowerBrownoutState`]: fires once on transition into
/// brownout and clears when the condition resolves, allowing re-fire.
pub fn tick_power_brownout_advisory(
    mut ships: Query<
        (
            Entity,
            &ShipPowerSystem,
            &mut PowerBrownoutState,
            Option<&PowerConfigResource>,
        ),
        With<crate::server_app::Ship>,
    >,
    config_res: Option<Res<PowerConfigResource>>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, power, mut brownout_state, config_comp) in ships.iter_mut() {
        let total = power.0.total();
        let cfg_default;
        let cfg: &PowerConfigResource = match config_comp {
            Some(c) => c,
            None => match config_res.as_deref() {
                Some(c) => c,
                None => {
                    cfg_default = PowerConfigResource::default();
                    &cfg_default
                }
            },
        };
        let rate = cfg
            .0
            .rates
            .get((total as usize).clamp(3, 8) - 3)
            .copied()
            .unwrap_or(0.0);
        let is_draining = rate < 0.0;

        let mut still_brownouting = std::collections::HashSet::new();

        for (group_id, level) in power.0.iter() {
            if is_draining && level > 1 {
                still_brownouting.insert(group_id.0.clone());

                // Rising edge: group was not previously notified → emit advisory.
                if !brownout_state.notified_groups.contains(&group_id.0) {
                    if let Some(sys_id) = system_id_for_power_group(&group_id.0) {
                        writer.write(CoordinationEnqueue {
                            source_entity: entity,
                            sender_origin: ControlSource::Ai,
                            target: sys_id,
                            payload: CoordinationPayload::PowerBrownout {
                                group: group_id.0.clone(),
                                label: power_group_label(&group_id.0).to_string(),
                                allocated_level: level,
                            },
                            sender_label: "Power".into(),
                        });
                    }
                }
            }
        }

        // Update notified set: groups still in brownout stay notified;
        // groups that cleared are removed (can re-fire on next cycle).
        brownout_state.notified_groups = still_brownouting;
    }
}

// ── Blackboard publish (issue #561) ──────────────────────────────────────────

fn publish_power_blackboard(
    power_res: Option<Res<ShipPowerSystem>>,
    config_res: Option<Res<PowerConfigResource>>,
    multipliers_res: Option<Res<PowerMultiplierResource>>,
    ship_q: Query<
        (
            &ShipPowerSystem,
            Option<&PowerConfigResource>,
            Option<&PowerMultiplierResource>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    control_sources_q: Query<
        &crate::ship_plugin::ShipSystemControlSources,
        With<crate::server_app::LocalShip>,
    >,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    use crate::system_registry::{
        POWER_BATTERY_SYSTEM_ID, POWER_REACTOR_SYSTEM_ID, POWER_SYSTEM_ID,
    };

    // Prefer per-entity components on LocalShip; fall back to global Resources.
    let entity_view = ship_q.single().ok();
    let power_default;
    let power: &ShipPowerSystem = match entity_view.map(|(p, _, _)| p) {
        Some(p) => p,
        None => match power_res.as_deref() {
            Some(p) => p,
            None => {
                power_default =
                    ShipPowerSystem(crate::modifiers::power_system::PowerSystem::default());
                &power_default
            }
        },
    };
    let config_default;
    let config: &PowerConfigResource = match entity_view.and_then(|(_, c, _)| c) {
        Some(c) => c,
        None => match config_res.as_deref() {
            Some(c) => c,
            None => {
                config_default = PowerConfigResource::default();
                &config_default
            }
        },
    };
    let multipliers_default;
    let multipliers: &PowerMultiplierResource = match entity_view.and_then(|(_, _, m)| m) {
        Some(m) => m,
        None => match multipliers_res.as_deref() {
            Some(m) => m,
            None => {
                multipliers_default = PowerMultiplierResource::default();
                &multipliers_default
            }
        },
    };
    // Per-fine-system online state, derived from offline_systems on the
    // LocalShip's control sources. Read via a separate query so the
    // fine-system online flags survive test setups that spawn a LocalShip
    // with `ShipSystemControlSources` but no per-entity `ShipPowerSystem`
    // component (the primary `ship_q` above requires the latter).
    let control_sources = control_sources_q.single().ok();
    let reactor_id = crate::system_registry::power_reactor_system_id();
    let battery_id = crate::system_registry::power_battery_system_id();
    let reactor_online = control_sources
        .map(|cs| !cs.0.offline_systems.contains(&reactor_id))
        .unwrap_or(true);
    let battery_online = control_sources
        .map(|cs| !cs.0.offline_systems.contains(&battery_id))
        .unwrap_or(true);

    let entries: Vec<PowerGroupEntry> = POWER_GROUP_ORDER
        .iter()
        .map(|name| PowerGroupId(name.to_string()))
        .filter(|gid| multipliers.multipliers.contains_key(gid))
        .map(|gid| {
            let max_level = multipliers
                .multipliers
                .get(&gid)
                .map(|arr| arr.len() as u8)
                .unwrap_or(4);
            PowerGroupEntry {
                id: gid.0.clone(),
                label: power_group_label(gid.0.as_str()).into(),
                level: power_level_for(&power.0, &gid),
                max_level,
            }
        })
        .collect();

    let bb = PowerBlackboard {
        groups: entries,
        total: power.0.total(),
        total_max: 8,
        battery_charge: power.0.battery_charge,
        battery_max: config.0.capacity,
        locked: power.0.locked,
    };
    // Fine blackboards (issue #513) — reactor owns the allocation surface,
    // battery owns the emergency-reserve pool. Emitted alongside the legacy
    // coarse `Power` blackboard so downstream JS panels can pick either or both.
    let reactor_bb = PowerReactorBlackboard {
        total_allocation: power.0.total(),
        max_allocation: 8,
        is_online: reactor_online,
        locked: power.0.locked,
    };
    let emergency_threshold = if config.0.capacity > 0.0 {
        (config.0.emergency_threshold / config.0.capacity).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let battery_bb = PowerBatteryBlackboard {
        charge: power.0.battery_charge,
        capacity: config.0.capacity,
        is_online: battery_online,
        emergency_threshold,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(POWER_SYSTEM_ID.to_string()),
            SystemBlackboard::Power(bb),
        );
        bbs.0.insert(
            SystemId(POWER_REACTOR_SYSTEM_ID.to_string()),
            SystemBlackboard::PowerReactor(reactor_bb),
        );
        bbs.0.insert(
            SystemId(POWER_BATTERY_SYSTEM_ID.to_string()),
            SystemBlackboard::PowerBattery(battery_bb),
        );
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
    use crate::messages::{ModifierSlot, ServerMessage, *};
    use crate::modifiers::ShipModifiers;
    use crate::power_system::SENSORS_POWER_GROUP;
    use crate::shield::ShieldSystem;
    use crate::simulation::{
        LastBroadcastEntityPositions, LastBroadcastHull, LastBroadcastShields, ShipImpulse,
        ShipShields, SimOutbox,
    };

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .add_plugins(ShipPowerPlugin)
            .add_systems(
                Update,
                crate::modifier_coordination::translate_power_modifiers.after(tick_power_system),
            )
            .add_plugins(crate::simulation::sim_state_broadcaster())
            .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_power_messages can query it.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            ShipShields(ShieldSystem::default(), 0.5),
            ShipModifiers::new(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            PowerBrownoutState::default(),
        ));
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

    fn start_game(app: &mut App) {
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
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn start_game_with_power(app: &mut App) {
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
            "power",
            ClientMessage::Identify {
                token: "power".into(),
                name: "Monty".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "power",
            ClientMessage::SelectStation {
                station: "Power".into(),
            },
        );
        tick(app);
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "power", ClientMessage::SetReady { ready: true });
        let _ = tick(app);
    }

    #[test]
    fn power_state_only_sent_to_power_holder() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let out = tick(&mut app);

        for m in out
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::PowerState { .. }))
        {
            assert!(
                matches!(&m.target, Target::Token(t) if t == "power"),
                "PowerState should only go to the Power holder, got {:?}",
                m.target
            );
        }
    }

    #[test]
    fn no_power_station_holder_no_power_state_broadcast() {
        let mut app = test_app();
        start_game(&mut app);

        let out = tick(&mut app);
        let any_power_state = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(
            !any_power_state,
            "no PowerState should be sent when no Power station holder exists"
        );
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 4);
        // Directly set via resource (the human message path lives in ship_plugin.rs).
        // Verify the field clamps at 4 on the PowerSystem itself.
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
            4,
            "helm should remain at 4"
        );
    }

    #[test]
    fn power_increase_respects_total_cap_of_eight() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Force total to 8 and check PowerSystem::increase is a no-op.
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            let _ = ps
                .0
                .set_group_allocation(&crate::messages::PowerGroupId(HELM_POWER_GROUP.into()), 4);
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(WEAPONS_POWER_GROUP.into()),
                2,
            );
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(SENSORS_POWER_GROUP.into()),
                2,
            );
        }
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.increase(&PowerGroupId(SENSORS_POWER_GROUP.into()));
        }
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SENSORS_POWER_GROUP.into())),
            2,
            "sensors should stay at 2 when total is already at the cap of 8"
        );
        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.total(),
            8,
            "total should remain 8"
        );
    }

    #[test]
    fn increasing_helm_power_updates_max_speed_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(PowerGroupId(HELM_POWER_GROUP.into()), [-0.5, 0.0, 1.0, 2.0]);

        // Directly set helm=3 and tick to let translate_power_modifiers run.
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 3);
        let _ = tick(&mut app);

        let mult = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipModifiers, With<crate::simulation::LocalShip>>();
            q.single(app.world()).unwrap().get(&ModifierSlot::MaxSpeed)
        };
        assert!(
            (mult - 2.0).abs() < 1e-6,
            "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}"
        );
    }

    #[test]
    fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                PowerGroupId(WEAPONS_POWER_GROUP.into()),
                [-0.5, 0.0, 0.25, 0.5],
            );

        // Set weapons=1 directly and tick.
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(&PowerGroupId(WEAPONS_POWER_GROUP.into()), 1);
        let _ = tick(&mut app);

        let expected = 1.0 / 1.5;
        let mult = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipModifiers, With<crate::simulation::LocalShip>>();
            q.single(app.world())
                .unwrap()
                .get(&ModifierSlot::PhaserDamage)
        };
        assert!(
            (mult - expected).abs() < 1e-6,
            "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}"
        );
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_updates_all_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let defaults = [-0.5, 0.0, 0.25, 0.5];
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(PowerGroupId(HELM_POWER_GROUP.into()), defaults);
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(PowerGroupId(WEAPONS_POWER_GROUP.into()), defaults);
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(PowerGroupId(SENSORS_POWER_GROUP.into()), defaults);

        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            let _ = ps
                .0
                .set_group_allocation(&crate::messages::PowerGroupId(HELM_POWER_GROUP.into()), 4);
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(WEAPONS_POWER_GROUP.into()),
                2,
            );
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(SENSORS_POWER_GROUP.into()),
                2,
            );
            ps.0.battery_charge = 0.0;
            ps.0.locked = false;
        }

        tick(&mut app);

        let expected = 1.0 / 1.5;
        let mods = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipModifiers, With<crate::simulation::LocalShip>>();
            q.single(app.world()).unwrap().clone()
        };

        assert!(
            (mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-6,
            "after exhaustion MaxSpeed should be {expected}, got {}",
            mods.get(&ModifierSlot::MaxSpeed)
        );
        assert!(
            (mods.get(&ModifierSlot::PhaserDamage) - expected).abs() < 1e-6,
            "after exhaustion PhaserDamage should be {expected}, got {}",
            mods.get(&ModifierSlot::PhaserDamage)
        );
        assert!(
            (mods.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6,
            "after exhaustion RadarRange should be {expected}, got {}",
            mods.get(&ModifierSlot::RadarRange)
        );
    }

    // ── Blackboard publish tests ────────────────────────────────────────────

    fn power_blackboard(app: &mut App) -> PowerBlackboard {
        use crate::messages::{SystemBlackboard, SystemId};
        use crate::server_app::{LocalShip, ShipSystemBlackboards};
        use crate::system_registry::POWER_SYSTEM_ID;
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        match bbs.0.get(&SystemId(POWER_SYSTEM_ID.to_string())) {
            Some(SystemBlackboard::Power(bb)) => bb.clone(),
            _ => PowerBlackboard::default(),
        }
    }

    #[test]
    fn publish_power_blackboard_contains_correct_data() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = power_blackboard(&mut app);
        assert!(
            !bb.groups.is_empty(),
            "expected at least one power group entry"
        );
        assert!(
            bb.groups.iter().any(|e| e.label == "HELM"),
            "expected HELM entry"
        );
        assert!(
            bb.groups.iter().any(|e| e.label == "WEAPONS"),
            "expected WEAPONS entry"
        );
        assert!(
            bb.groups.iter().any(|e| e.label == "SENSORS"),
            "expected SENSORS entry"
        );
        assert!(bb.total > 0, "total should be > 0");
        assert!(!bb.locked, "should not be locked initially");
    }

    #[test]
    fn publish_power_blackboard_reflects_helm_level_change() {
        let mut app = test_app();
        // Human holds Power so operate_power_ai yields and doesn't override.
        start_game_with_power(&mut app);
        tick(&mut app);

        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 3);
        tick(&mut app);

        let bb = power_blackboard(&mut app);
        let helm_entry = bb.groups.iter().find(|e| e.label == "HELM").unwrap();
        assert_eq!(
            helm_entry.level, 3,
            "helm level should be 3 after direct assignment"
        );
    }

    #[test]
    fn control_system_set_power_group_allocation_updates_group() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        push_msg(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(SENSORS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SENSORS_POWER_GROUP.into())),
            4
        );
    }

    /// Wire-string regression: JS clients send `target: 'power-reactor'`
    /// (see `gui/action-map.js` `set_power` handler). This test pins the
    /// exact string used on the wire, so if either the JS side or the
    /// handler's `for_target(...)` argument drifts back to `"power"`,
    /// this fails (the admitted command routes elsewhere and the
    /// allocation never applies).
    #[test]
    fn set_power_group_allocation_wire_string_routes_to_reactor() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        push_msg(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: SystemId("power-reactor".to_string()),
                payload: SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(SENSORS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SENSORS_POWER_GROUP.into())),
            4,
            "raw wire string \"power-reactor\" must reach handle_power_messages \
             — if this fails, either the handler's for_target() argument or the \
             JS action-map target has drifted from \"power-reactor\"."
        );
    }

    // ── operate_power_ai tests ──────────────────────────────────────────────

    fn ai_test_app() -> App {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerAiConfigResource>()
            .add_systems(Update, operate_power_ai);

        // Spawn a Ship entity with ShipPowerSystem component + AI power source.
        let mut resolver = ControlSourceResolver::new();
        resolver.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipSystemControlSources(resolver),
            crate::ship_plugin::ShipConfigComponent::default(),
            ShipPowerSystem(PowerSystem::default()),
            crate::ship_state::ShipRedAlert::default(),
            LastHelmInput::default(),
        ));
        app
    }

    #[test]
    fn ai_sets_helm_to_three_when_high_throttle_and_battery_ok() {
        let mut app = ai_test_app();
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(ship).insert(LastHelmInput {
            thrust: 0.9,
            steering: 0.0,
            lateral: 0.0,
        });
        // battery_pct = 100/100 = 1.0 >= 0.75 floor
        app.update();
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
            3
        );
    }

    #[test]
    fn ai_sets_helm_to_one_when_throttle_is_zero() {
        let mut app = ai_test_app();
        // Default LastHelmInput has thrust=0.0
        app.update();
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
            1
        );
    }

    #[test]
    fn ai_sets_weapons_to_three_on_red_alert_with_battery() {
        let mut app = ai_test_app();
        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_state::ShipRedAlert, bevy::prelude::With<crate::simulation::LocalShip>>();
            if let Ok(mut ra) = q.single_mut(app.world_mut()) {
                ra.toggle();
            }
        }
        app.update();
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(WEAPONS_POWER_GROUP.into())),
            3
        );
    }

    #[test]
    fn ai_does_not_boost_weapons_when_battery_low() {
        let mut app = ai_test_app();
        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_state::ShipRedAlert, bevy::prelude::With<crate::simulation::LocalShip>>();
            if let Ok(mut ra) = q.single_mut(app.world_mut()) {
                ra.toggle();
            }
        }
        // Set battery low on both the resource and the component.
        app.world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .battery_charge = 30.0; // pct=0.3 < 0.5 floor
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPowerSystem, With<crate::server_app::LocalShip>>();
            let mut ps = q.single_mut(app.world_mut()).unwrap();
            ps.0.battery_charge = 30.0;
        }
        app.update();
        // weapons should not be 3 — battery below floor
        assert_ne!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(WEAPONS_POWER_GROUP.into())),
            3,
            "weapons should not be boosted when battery is below floor"
        );
    }

    // ── Inter-system command channel tests (issue #559) ───────────────────────
    //
    // These tests exercise the full weapons→power inter-system drain flow.
    // A minimal combined app registers both `drain_power_for_active_beam`
    // (Weapons, Physics) and `handle_power_inter_system` (Power, Modifiers)
    // with SimSets chained, so we can set an active beam and verify the
    // Power battery decreases in the same tick.

    fn inter_system_test_app() -> App {
        use crate::console::weapons::server::{drain_power_for_active_beam, ActiveBeam};
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            // Power resources and handler.
            .insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<crate::messages::InterSystemQueue>()
            // Chain emitter before consumer so the queue is populated before it's read.
            .add_systems(
                Update,
                (drain_power_for_active_beam, handle_power_inter_system).chain(),
            );
        // Spawn a LocalShip entity carrying the per-entity ActiveBeam component.
        // After PR-7 (issue #597) `ActiveBeam` is a per-entity `Component`, not a `Resource`.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            ActiveBeam::default(),
        ));
        // Warm up the time plugin so delta_secs is non-zero on the first real tick.
        app.update();
        app
    }

    /// Helper: mutate the LocalShip's `ActiveBeam` component in tests.
    fn set_beam_target(app: &mut App, uuid: Option<String>) {
        use crate::console::weapons::server::ActiveBeam;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::simulation::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.target_uuid = uuid;
        }
    }

    #[test]
    fn active_beam_drains_power_battery_via_inter_system_channel() {
        use crate::console::weapons::server::PHASER_BATTERY_DRAIN_PER_SEC;
        let mut app = inter_system_test_app();

        // Simulate an active phaser beam.
        set_beam_target(&mut app, Some("target-asteroid".into()));

        let charge_before = app.world().resource::<ShipPowerSystem>().0.battery_charge;
        app.update();
        let charge_after = app.world().resource::<ShipPowerSystem>().0.battery_charge;

        // dt = 100ms → expected drain ≈ PHASER_BATTERY_DRAIN_PER_SEC * 0.1
        let expected_drain = PHASER_BATTERY_DRAIN_PER_SEC * 0.1;
        assert!(
            charge_after < charge_before,
            "active beam must drain battery (before={charge_before}, after={charge_after})"
        );
        assert!(
            (charge_before - charge_after - expected_drain).abs() < 0.1,
            "drain should be ~{expected_drain} (before={charge_before}, after={charge_after})"
        );
    }

    #[test]
    fn no_beam_does_not_drain_power_battery() {
        let mut app = inter_system_test_app();
        // ActiveBeam defaults to target_uuid = None.
        let charge_before = app.world().resource::<ShipPowerSystem>().0.battery_charge;
        app.update();
        let charge_after = app.world().resource::<ShipPowerSystem>().0.battery_charge;

        assert_eq!(
            charge_before, charge_after,
            "no active beam must not drain battery (before={charge_before}, after={charge_after})"
        );
    }

    #[test]
    fn inter_system_drain_clamps_battery_to_zero() {
        let mut app = inter_system_test_app();

        // Set battery nearly empty (less than one tick of drain).
        app.world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .battery_charge = 0.1;
        set_beam_target(&mut app, Some("target-asteroid".into()));

        app.update();

        let charge = app.world().resource::<ShipPowerSystem>().0.battery_charge;
        assert_eq!(
            charge, 0.0,
            "battery must clamp at zero, not go negative (got {charge})"
        );
    }

    // ── Fine Power system tests (issue #513) ───────────────────────────────────
    //
    // Cover the reactor / battery offline gates and the fine-system blackboard
    // publication. Uses the same inter_system_test_app scaffold but adds
    // ShipSystemControlSources so `offline_systems` can be seeded.

    /// Variant of `inter_system_test_app` whose Ship entity carries
    /// `ShipSystemControlSources` so tests can seed `offline_systems`
    /// (mirrors what `sync_console_damage_tiers` would do on Disabled hull).
    fn inter_system_test_app_with_control_sources() -> App {
        use crate::console::weapons::server::{drain_power_for_active_beam, ActiveBeam};
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<crate::messages::InterSystemQueue>()
            .add_systems(
                Update,
                (drain_power_for_active_beam, handle_power_inter_system).chain(),
            );
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            ActiveBeam::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
        ));
        app.update(); // warm up TimePlugin
        app
    }

    fn set_beam_target_on(app: &mut App, uuid: Option<String>) {
        use crate::console::weapons::server::ActiveBeam;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::simulation::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.target_uuid = uuid;
        }
    }

    fn mark_offline(app: &mut App, system_id: SystemId) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut cs = app
            .world_mut()
            .entity_mut(ship)
            .take::<crate::ship_plugin::ShipSystemControlSources>()
            .unwrap();
        cs.0.offline_systems.insert(system_id);
        app.world_mut().entity_mut(ship).insert(cs);
    }

    #[test]
    fn reactor_offline_refuses_allocation_input() {
        // End-to-end path via handle_power_messages: the admission gate
        // ensures a Disabled/Destroyed reactor's `accept_human_input` is
        // false, but the direct test is that dispatching a SetPowerGroupAllocation
        // to the reactor id when it's offline leaves battery/allocation untouched.
        //
        // We test the handler directly (bypassing admission which lives in
        // server_app.rs) by seeding an AdmittedCommand targeting the
        // reactor's id and verifying the mutation still applies when the
        // system is online, then does NOT apply when offline_systems marks
        // the reactor offline via the standard admission gate.
        //
        // Since `handle_power_messages` does not itself consult
        // `offline_systems` (admission does), we cover this via the
        // full admission chain in a mini test app that includes
        // `AdmissionPlugin`.
        use crate::lobby::LobbyPlugin;
        use crate::messages::{ClientMessage, PowerGroupId, SystemControlPayload};
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .add_plugins(ShipPowerPlugin)
            .add_plugins(crate::simulation::sim_state_broadcaster());
        // Spawn the player ship with control sources so we can seed offline_systems.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            ShipModifiers::new(),
            PowerBrownoutState::default(),
        ));
        start_game_with_power(&mut app);
        // Baseline: reactor online — allocation should update.
        push_msg(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: SystemControlPayload::SetPowerGroupAllocation {
                    group: PowerGroupId(SENSORS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SENSORS_POWER_GROUP.into())),
            4,
            "baseline sanity: online reactor should accept allocation input"
        );

        // Now mark the reactor offline and try to change sensors back to 1.
        mark_offline(&mut app, crate::system_registry::power_reactor_system_id());
        push_msg(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: SystemControlPayload::SetPowerGroupAllocation {
                    group: PowerGroupId(SENSORS_POWER_GROUP.into()),
                    level: 1,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SENSORS_POWER_GROUP.into())),
            4,
            "reactor offline must refuse allocation input (sensors should stay at 4)"
        );
    }

    #[test]
    fn battery_offline_refuses_drain_from_channel_2() {
        let mut app = inter_system_test_app_with_control_sources();
        // Mark battery offline via offline_systems (mirrors sync_console_damage_tiers).
        mark_offline(&mut app, crate::system_registry::power_battery_system_id());

        set_beam_target_on(&mut app, Some("target-asteroid".into()));
        // Snapshot both the Resource and the per-entity Component charge; the
        // ship spawn used here has no ShipPowerSystem component so the fallback
        // Resource path is what gets exercised. Set the resource charge to a
        // known baseline so we can verify no change.
        app.world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .battery_charge = 50.0;

        // Also ensure the per-entity charge (if any) matches.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPowerSystem, With<crate::simulation::LocalShip>>();
            if let Ok(mut pc) = q.single_mut(app.world_mut()) {
                pc.0.battery_charge = 50.0;
            }
        }

        app.update();

        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.battery_charge,
            50.0,
            "battery offline must refuse channel-2 drain (charge should stay at 50.0)"
        );
    }

    #[test]
    fn publish_writes_power_reactor_and_power_battery_blackboards() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        use crate::server_app::{LocalShip, ShipSystemBlackboards};
        use crate::system_registry::{POWER_BATTERY_SYSTEM_ID, POWER_REACTOR_SYSTEM_ID};
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let bbs = q.single(app.world()).unwrap();

        let reactor = bbs.0.get(&SystemId(POWER_REACTOR_SYSTEM_ID.to_string()));
        let battery = bbs.0.get(&SystemId(POWER_BATTERY_SYSTEM_ID.to_string()));
        assert!(
            matches!(reactor, Some(SystemBlackboard::PowerReactor(_))),
            "expected PowerReactor blackboard under power-reactor system id, got {reactor:?}"
        );
        assert!(
            matches!(battery, Some(SystemBlackboard::PowerBattery(_))),
            "expected PowerBattery blackboard under power-battery system id, got {battery:?}"
        );
        if let Some(SystemBlackboard::PowerReactor(bb)) = reactor {
            assert!(
                bb.is_online,
                "reactor is_online must default to true when nothing is marked offline"
            );
        }
        if let Some(SystemBlackboard::PowerBattery(bb)) = battery {
            assert!(
                bb.is_online,
                "battery is_online must default to true when nothing is marked offline"
            );
        }
    }

    #[test]
    fn reactor_offline_blackboard_reports_is_online_false() {
        let mut app = test_app();
        start_game(&mut app);
        // Seed offline_systems on the ship's control sources.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut cs = app
                .world_mut()
                .entity_mut(ship)
                .take::<crate::ship_plugin::ShipSystemControlSources>()
                .unwrap();
            cs.0.offline_systems
                .insert(crate::system_registry::power_reactor_system_id());
            app.world_mut().entity_mut(ship).insert(cs);
        }
        tick(&mut app);

        use crate::server_app::{LocalShip, ShipSystemBlackboards};
        use crate::system_registry::POWER_REACTOR_SYSTEM_ID;
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        match bbs.0.get(&SystemId(POWER_REACTOR_SYSTEM_ID.to_string())) {
            Some(SystemBlackboard::PowerReactor(bb)) => {
                assert!(
                    !bb.is_online,
                    "reactor blackboard is_online must be false when offline_systems contains power-reactor"
                );
            }
            other => panic!("expected PowerReactor blackboard, got {other:?}"),
        }
    }

    #[test]
    fn battery_offline_blackboard_reports_is_online_false() {
        let mut app = test_app();
        start_game(&mut app);
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut cs = app
                .world_mut()
                .entity_mut(ship)
                .take::<crate::ship_plugin::ShipSystemControlSources>()
                .unwrap();
            cs.0.offline_systems
                .insert(crate::system_registry::power_battery_system_id());
            app.world_mut().entity_mut(ship).insert(cs);
        }
        tick(&mut app);

        use crate::server_app::{LocalShip, ShipSystemBlackboards};
        use crate::system_registry::POWER_BATTERY_SYSTEM_ID;
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        match bbs.0.get(&SystemId(POWER_BATTERY_SYSTEM_ID.to_string())) {
            Some(SystemBlackboard::PowerBattery(bb)) => {
                assert!(
                    !bb.is_online,
                    "battery blackboard is_online must be false when offline_systems contains power-battery"
                );
            }
            other => panic!("expected PowerBattery blackboard, got {other:?}"),
        }
    }

    #[test]
    fn power_state_broadcast_still_sends_to_power_holder_when_reactor_offline() {
        let mut app = test_app();
        start_game_with_power(&mut app);
        // Mark the reactor offline (audience routing should not care).
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut cs = app
                .world_mut()
                .entity_mut(ship)
                .take::<crate::ship_plugin::ShipSystemControlSources>()
                .unwrap();
            cs.0.offline_systems
                .insert(crate::system_registry::power_reactor_system_id());
            app.world_mut().entity_mut(ship).insert(cs);
        }

        let out = tick(&mut app);
        // At least one PowerState message should still be sent to the power holder.
        let power_state_to_power_holder = out.iter().any(|m| {
            matches!(&m.msg, ServerMessage::PowerState { .. })
                && matches!(&m.target, Target::Token(t) if t == "power")
        });
        assert!(
            power_state_to_power_holder,
            "PowerState broadcast must still target the Power holder even when the reactor is offline"
        );
    }

    // ── Brownout advisory tests (issue #678) ────────────────────────────────

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

    fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
        let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
        app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
        msgs
    }

    fn brownout_test_app() -> App {
        let mut app = test_app();
        // Insert ShipPowerSystem component on the LocalShip entity so
        // tick_power_brownout_advisory's query matches it.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .insert(ShipPowerSystem(PowerSystem::default()));
        app.init_resource::<CoordEnqueueBox>()
            .add_systems(PostUpdate, collect_coord);
        app
    }

    #[test]
    fn tick_power_brownout_advisory_emits_on_drain_and_debounces() {
        let mut app = brownout_test_app();
        start_game(&mut app);

        // Helper: mutate the per-entity ShipPowerSystem component (the
        // advisory system reads from the component, not the resource).
        fn set_ship_power(app: &mut App, group: &str, level: u8) {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPowerSystem, With<crate::simulation::LocalShip>>();
            if let Ok(mut ps) = q.single_mut(app.world_mut()) {
                let _ =
                    ps.0.set_group_allocation(&PowerGroupId(group.into()), level);
            }
        }

        // Tick 1: default total=6, not draining (rate=2.0 > 0) → no advisory.
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert!(
            emitted.is_empty(),
            "no advisory when total=6 (not draining): got {}",
            emitted.len()
        );

        // Set total=7 (helm up to 3, weapons=2, sensors=2).
        // With default rates [6,5,4,2,-2,-6], total=7 → rate=-2.0 → draining.
        // All three groups have level > 1 → all three emit advisories.
        set_ship_power(&mut app, HELM_POWER_GROUP, 3);
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert_eq!(
            emitted.len(),
            3,
            "three PowerBrownout advisories (one per group) when draining at total=7"
        );
        for e in &emitted {
            assert!(
                matches!(&e.payload, CoordinationPayload::PowerBrownout { .. }),
                "expected PowerBrownout, got {:?}",
                e.payload
            );
        }

        // Tick 2: still draining → debounce holds → no re-emission.
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert!(
            emitted.is_empty(),
            "debounce: no re-emission while condition persists"
        );

        // Reset helm to 2 → total=6, condition clears.
        set_ship_power(&mut app, HELM_POWER_GROUP, 2);
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert!(emitted.is_empty(), "no advisory when condition clears");

        // Re-enter drain (total=7 again) → re-fire allowed (debounce cleared).
        set_ship_power(&mut app, HELM_POWER_GROUP, 3);
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert_eq!(
            emitted.len(),
            3,
            "re-fire: three advisories re-emitted after clear-and-return"
        );

        // Clear again, then set sensors=1 (level 1, idle) alongside
        // weapons=3 and helm=3 → only weapons and helm should fire.
        set_ship_power(&mut app, HELM_POWER_GROUP, 2);
        let _ = tick(&mut app);
        let _ = drain_coord(&mut app); // flush any stale events
        set_ship_power(&mut app, HELM_POWER_GROUP, 3);
        set_ship_power(&mut app, WEAPONS_POWER_GROUP, 3);
        set_ship_power(&mut app, SENSORS_POWER_GROUP, 1);
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert_eq!(
            emitted.len(),
            2,
            "two advisories: weapons and helm (level 3), sensors at 1 should not fire"
        );
        for e in &emitted {
            match &e.payload {
                CoordinationPayload::PowerBrownout { group, .. } => {
                    assert_ne!(
                        group.as_str(),
                        SENSORS_POWER_GROUP,
                        "sensors at level 1 must not get a brownout advisory"
                    );
                }
                _ => panic!("unexpected payload type"),
            }
        }
    }
}

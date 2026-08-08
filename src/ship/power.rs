use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{
    CoordinationPayload, InterSystemPayload, InterSystemQueue, PowerBatteryBlackboard,
    PowerBlackboard, PowerGroupEntry, PowerGroupId, PowerReactorBlackboard, ServerMessage,
    SystemBlackboard, SystemId,
};
use crate::modifiers::power_system::{
    power_level_for_group, PowerConfig, PowerSystem, HELM_POWER_GROUP, POWER_GROUP_ORDER,
    SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
};
use crate::ship_plugin::CoordinationEnqueue;

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

// ── AI policy (issue #784) ──────────────────────────────────────────────────

/// Per-ship inline stateless Power allocation policy (issue #784).
///
/// From the ship's `[power.ai_policy]` block when authored, otherwise the
/// canonical [`crate::entities::config::default_power_ai_config`] policy
/// (which reproduces the retired stateful engine's helm←thrust / weapons←red
/// alert behaviour with reserve guards). Read by
/// `console_ai::server::ai_power_allocation`, which for each of the ship's
/// AUTHORED power groups resolves that group's channel over an immutable
/// per-tick fact snapshot and emits the winning `set_power_group_allocation`
/// verb's absolute level as an admitted `SetPowerGroupAllocation` — the same
/// admitted command a human Power operator sends.
///
/// This RETIRES the bespoke stateful `PowerAiConfigResource` +
/// `ShipPowerAiState` (`EngageState` hysteresis) from #762: the decision is now
/// a pure function of the per-tick snapshot, with no private timer state
/// (AGENTS.md rule #7). Attached to every ship at spawn (player + NPC).
#[derive(Component, Clone, Debug, Default)]
pub struct PowerAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship multiplier on the shared AI base cadence for `[power.ai_policy]`
/// (issue #889's PASM-tracked runtime gap: `evaluate_every_ticks` was parsed
/// and validated but no host read it). `ai_power_allocation` decides on every
/// Nth arm of the shared `ai_tick_ready` latch rather than every arm.
///
/// A sibling component to [`PowerAiPolicy`] rather than a field ON it (or on
/// the shared [`crate::ai::policy::AiPolicy`] type itself): dozens of call
/// sites across the crate construct an `AiPolicy` by literal, and this way
/// wiring Power's cadence costs one small component instead of touching every
/// one of them, most of which do not (yet) need per-host cadence at all.
///
/// `1` — the parse default, and what every shipped hull authors today — means
/// "every arm", identical to behaviour before this component existed.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PowerAiCadence(pub u32);

/// Seed the immutable per-tick fact snapshot the Power allocation policy
/// resolves each authored power group's channel against (issue #784).
///
/// Pure and Bevy-free (AGENTS.md rule #10): the host reads live state and passes
/// primitives in. Seeds the BROAD fact set so authored guards can read ship,
/// threat, objective, and system facts — but the canonical default policy only
/// needs `battery_pct`, `thrust`, and `red_alert`. The #779 empty-facts lesson:
/// a `fact(...)` guard validates but never fires unless the host seeds the fact,
/// so this is THE piece that makes the reserve guard (and every other) live.
///
/// SHIP facts: `battery_pct`, `thrust`, `red_alert`, per-group `power_<group>`
/// current level, and `total_allocation`. THREAT: `secs_since_combat` (absent
/// when the ship has no combat history) and `nearest_enemy_dist` (absent when
/// none is known). OBJECTIVE: `has_destroy_objective` (`1.0`/`0.0`). SYSTEM:
/// `offline_system_count`.
#[allow(clippy::too_many_arguments)]
pub fn seed_power_facts(
    power: &PowerSystem,
    battery_pct: f32,
    thrust: f32,
    red_alert: bool,
    secs_since_combat: Option<f32>,
    nearest_enemy_dist: Option<f32>,
    has_destroy_objective: bool,
    offline_system_count: u32,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set(
        crate::entities::config::POWER_BATTERY_PCT_FACT,
        battery_pct as f64,
    );
    facts.set(crate::entities::config::POWER_THRUST_FACT, thrust as f64);
    facts.set(
        crate::entities::config::POWER_RED_ALERT_FACT,
        if red_alert { 1.0 } else { 0.0 },
    );
    // Per-group current level, keyed `power_<group>`, plus the ship-wide total.
    for (id, level) in power.iter() {
        facts.set(&format!("power_{}", id.0), level as f64);
    }
    facts.set("total_allocation", power.total() as f64);
    // THREAT: only set when known, so an absent-fact guard reads "no threat".
    if let Some(s) = secs_since_combat {
        facts.set("secs_since_combat", s as f64);
    }
    if let Some(d) = nearest_enemy_dist {
        facts.set("nearest_enemy_dist", d as f64);
    }
    // OBJECTIVE + SYSTEM.
    facts.set(
        "has_destroy_objective",
        if has_destroy_objective { 1.0 } else { 0.0 },
    );
    facts.set("offline_system_count", offline_system_count as f64);
    facts
}

/// Debounce state for power brownout coordination advisories (issue #678).
///
/// Tracks which power groups have already been notified of a brownout
/// condition so the advisory only fires once per transition into brownout.
/// Cleared when the group exits the brownout condition, allowing re-fire
/// on subsequent brownout cycles.
#[derive(Component, Default, Clone)]
pub struct PowerBrownoutState {
    /// Group id strings (e.g. "weapons", "helm", "shields") that are
    /// currently in a notified-brownout state.
    pub notified_groups: std::collections::HashSet<String>,
    /// Set by [`tick_power_system`] from
    /// [`crate::modifiers::power_system::PowerSystem::tick`]'s return whenever
    /// the set of floored groups changed this tick, and consumed (and cleared)
    /// by [`tick_power_brownout_advisory`] later in the same tick.
    ///
    /// This is the edge-triggered half of the advisory (issue #952). The
    /// debounce below is level-triggered off the reserve's direction, and on
    /// its own it hides the single most important event on the bus: the tick a
    /// group is CUT is also the tick the draw drops, which very often flips the
    /// reactor net-positive — so `is_draining` goes false and the advisory
    /// clears at the exact moment Helm or Tactical needed telling. The edge
    /// re-arms the debounce so the cut announces itself.
    pub floors_changed: bool,
}

/// Maps a canonical power group id string to its display label in the HTML
/// power panel. Anything unknown falls back to `"UNKNOWN"`.
///
/// **These literals are the live path, for every hull.** This function is the
/// sole producer of [`PowerGroupEntry::label`], nothing in `gui/` reads a
/// `power_groups` block, and no other code path resolves an authored
/// `[power_groups.<id>] label` — so what a Power officer sees on any hull is
/// "HELM" / "WEAPONS" / "SHIELDS" from right here. The authored `label` fields
/// are real `strings.csv` ids and are covered by `scripts/check-strings.mjs`,
/// but nothing displays them yet; wiring them up means carrying the ship's
/// authored labels into `publish_power_blackboard` and resolving them client
/// side through `t()`, with these as the fallback for a hull that authors none.
pub fn power_group_label(group_id: &str) -> &'static str {
    match group_id {
        HELM_POWER_GROUP => "HELM",
        WEAPONS_POWER_GROUP => "WEAPONS",
        SHIELDS_POWER_GROUP => "SHIELDS",
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

/// Build a deterministic seed list for
/// [`PowerSystem::from_authored_groups`](crate::modifiers::power_system::PowerSystem::from_authored_groups)
/// from a ship's authored `[power_groups.*]` config (issue #762).
///
/// The canonical groups (`helm`, `weapons`, `shields`) come first in their
/// stable [`POWER_GROUP_ORDER`], then any extra authored groups (e.g. `ops`)
/// sorted by id, each seeded at its authored `default_level`. Returns an empty
/// vec when there are no authored groups so the caller falls back to the
/// canonical default seeding (unchanged behaviour for NPCs / fixtures without a
/// `[power_groups.*]` block).
pub fn authored_power_group_seed(
    power_groups: &std::collections::HashMap<PowerGroupId, crate::ship::config::PowerGroupConfig>,
) -> Vec<(PowerGroupId, u8)> {
    if power_groups.is_empty() {
        return Vec::new();
    }
    let mut seed: Vec<(PowerGroupId, u8)> = Vec::with_capacity(power_groups.len());
    for &name in POWER_GROUP_ORDER {
        let id = PowerGroupId(name.to_string());
        if let Some(cfg) = power_groups.get(&id) {
            seed.push((id, cfg.default_level));
        }
    }
    let mut extra: Vec<(&PowerGroupId, &crate::ship::config::PowerGroupConfig)> = power_groups
        .iter()
        .filter(|(id, _)| !POWER_GROUP_ORDER.contains(&id.0.as_str()))
        .collect();
    extra.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (id, cfg) in extra {
        seed.push((id.clone(), cfg.default_level));
    }
    seed
}

/// Build the per-group battery-floor table (issue #952) a ship's
/// [`PowerConfig`] ticks against.
///
/// Two authored sources meet here, and they are separate on purpose:
///
/// * WHEN a group is cut — `[power.battery_floor] <group> = <percent>`, which
///   lives with the battery it thresholds so a hull can retune the whole ladder
///   in one block.
/// * WHERE it lands — that group's own `[power_groups.<group>] min_level`,
///   which already existed as its allocation lower bound and is exactly the
///   right answer: a brownout must not push a group below a level its own file
///   says it can never sit under.
///
/// A group named in `battery_floor_pct` with no `[power_groups.*]` entry floors
/// to [`crate::modifiers::power_system::UNAUTHORED_FLOOR_LEVEL`] — the level the
/// runtime seeds such a group at. This is the NPC case: the six hulls that
/// declare no power groups at all get the canonical trio seeded at 2, and a
/// brownout takes those ships back to nominal rather than below it. Landing
/// them on `PowerGroupConfig`'s `min_level` parse default of 1 instead would
/// have been a fleet-wide combat debuff dressed up as a brownout, on hulls
/// whose files say nothing about the subject.
pub fn authored_power_group_floors(
    battery_floor_pct: &std::collections::HashMap<String, f32>,
    power_groups: &std::collections::HashMap<PowerGroupId, crate::ship::config::PowerGroupConfig>,
) -> std::collections::HashMap<String, crate::modifiers::power_system::PowerGroupFloor> {
    battery_floor_pct
        .iter()
        .map(|(id, pct)| {
            let min_level = power_groups
                .get(&PowerGroupId(id.clone()))
                .map(|g| g.min_level)
                .unwrap_or(crate::modifiers::power_system::UNAUTHORED_FLOOR_LEVEL);
            (
                id.clone(),
                crate::modifiers::power_system::PowerGroupFloor {
                    battery_pct: *pct,
                    min_level,
                },
            )
        })
        .collect()
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipPowerPlugin;

impl Plugin for ShipPowerPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_power_messages` reads
        // the `power-reactor` system's admitted commands. (The `power-battery`
        // read in `handle_power_inter_system` is off the InterSystemQueue, a
        // separate bus outside admitted routing.)
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::POWER_REACTOR_SYSTEM_ID,
        ));
        app.init_resource::<crate::messages::InterSystemQueue>()
            .add_message::<CoordinationEnqueue>();
        app.insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .add_systems(
                FixedUpdate,
                (
                    // In `SimSet::Physics`, not Input (issue #831, mirroring
                    // shields #826): `admit_system_commands` clears every
                    // ship's `AdmittedCommands` before Input each tick, and the
                    // AI decide system (`console_ai::server::ai_power_allocation`,
                    // Physics) refills it same-tick via `validate_and_admit` —
                    // so the applier must consume in Physics *after* the AI emit
                    // or AI power commands would be silently lost. `ConsoleAiPlugin`
                    // declares the explicit
                    // `ai_power_allocation.before(handle_power_messages)` edge.
                    //
                    // `tick_power_system` is ALSO in Physics and also takes
                    // `&mut ShipPowerSystem`, so set membership alone leaves
                    // their order unspecified. The explicit
                    // `.before(tick_power_system)` edge restores the guarantee
                    // the old Input placement gave for free: a same-tick
                    // reallocation is applied before this tick's battery
                    // integration reads `total()`. (Unlike shields, whose
                    // `tick_shields` lives in the later `Modifiers` set, so set
                    // ordering sufficed there.)
                    handle_power_messages
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .before(tick_power_system),
                    tick_power_system.in_set(crate::sim_sets::SimSet::Physics),
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
            // Same component-then-Resource preference for the reactor config,
            // which `draining` needs to read this hull's own `rates`.
            let mut cq =
                world.query_filtered::<&PowerConfigResource, With<crate::server_app::LocalShip>>();
            let config = cq
                .iter(world)
                .next()
                .cloned()
                .or_else(|| world.get_resource::<PowerConfigResource>().cloned())
                .unwrap_or_default();
            vec![ServerMessage::PowerState {
                helm: power.0.level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
                weapons: power.0.level_for(&PowerGroupId(WEAPONS_POWER_GROUP.into())),
                shields: power.0.level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
                battery_charge: power.0.battery_charge,
                draining: power.0.is_draining(&config.0),
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
    // Iterate every ship (player + NPC) so the player's Power console commands
    // AND NPC AI reallocation both re-allocate that ship's own power grid
    // through the one applier. Since issue #831 the NPC AI path emits an
    // admitted `SetPowerGroupAllocation` (from
    // `console_ai::server::ai_power_allocation`) that lands in this same
    // `AdmittedCommands` queue — there is no longer a separate
    // `integrate_power_state` adapter mutating `ShipPowerSystem` directly.
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
            if cs.0.is_offline(&battery_id) {
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
///
/// Forwards [`PowerSystem::tick`]'s floor-set edge into
/// [`PowerBrownoutState::floors_changed`] on the same ship, for
/// [`tick_power_brownout_advisory`] to consume later in the tick.
pub fn tick_power_system(
    time: Res<Time>,
    mut power_res: Option<ResMut<ShipPowerSystem>>,
    config_res: Option<Res<PowerConfigResource>>,
    mut ships: Query<
        (
            &mut ShipPowerSystem,
            Option<&PowerConfigResource>,
            Option<&mut PowerBrownoutState>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let dt = time.delta_secs();
    let mut ticked_any = false;
    for (mut power, config_comp, brownout_state) in ships.iter_mut() {
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
        let floors_changed = power.0.tick(dt, &cfg.0);
        if let Some(mut state) = brownout_state {
            if floors_changed {
                state.floors_changed = true;
            }
        }
        ticked_any = true;
    }
    // Fallback: no ship entity with the component (test environments that only
    // insert the Resource form). Tick the Resource directly. The floor edge is
    // dropped here on purpose — there is no ship entity, so there is no
    // `PowerBrownoutState` to carry it to and no advisory system to read it.
    if !ticked_any {
        if let (Some(power), Some(config)) = (power_res.as_deref_mut(), config_res.as_deref()) {
            let _ = power.0.tick(dt, &config.0);
        }
    }
}

// ── Power brownout advisory (issue #678) ─────────────────────────────────────
//
// The old fused `operate_power_ai` (absolute-set, non-timer, non-
// AiHighFidelity-gated) was removed in issue #693. It is replaced by
// `console_ai::server::ai_power_allocation`, which since issue #831 emits an
// admitted `SetPowerGroupAllocation` applied by `handle_power_messages` above
// — the single applier for the human and AI paths alike (the intermediate
// `PowerReactorIntents` + `integrate_power_state` adapter has been retired).

/// Map a power group id string to the target `SystemId` for coordination.
fn system_id_for_power_group(group: &str) -> Option<SystemId> {
    match group {
        WEAPONS_POWER_GROUP => Some(crate::system_registry::tactical_station_key()),
        HELM_POWER_GROUP => Some(crate::system_registry::helm_station_key()),
        SHIELDS_POWER_GROUP => Some(crate::system_registry::shields_system_id()),
        _ => None,
    }
}

/// Emit power brownout coordination advisories for groups with active demand
/// that cannot be satisfied (total allocation > 6 → battery draining).
///
/// An advisory fires **only** when:
/// - The reactor is net-negative at the current draw
///   (`PowerSystem::is_draining`) OR the group is currently being held down by
///   its authored battery floor (`PowerSystem::is_floored`)
/// - The group's EFFECTIVE allocation level > 1 (system is actively drawing,
///   not idle)
///
/// The `is_floored` half of the first condition is not redundant with the
/// draining half — it is the case the draining half structurally cannot see.
/// Cutting a group LOWERS the effective total, which is what picks the `rates`
/// rung, so the cut very often makes the reactor net-POSITIVE: on a
/// draining-only test the advisory would clear on precisely the tick Helm or
/// Tactical lost its power. A group held below its command is in a brownout by
/// any useful definition, whatever the reserve is doing.
///
/// The second condition is why the retired brownout lock used to silence every
/// advisory at once: it slammed all three groups to 1 together. Since issue
/// #952 a group is held at its own authored floor instead, which on the shipped
/// hulls is NOMINAL for helm and weapons — so a browned-out ship goes on
/// advising, correctly: those groups are still drawing, they have simply
/// stopped drawing the extra. A group whose hull authors `min_level = 1` does
/// fall silent, one at a time, as its own floor bites.
///
/// Debounced via [`PowerBrownoutState`]: fires once on transition into
/// brownout and clears when the condition resolves, allowing re-fire. The
/// debounce is additionally re-armed by
/// [`PowerBrownoutState::floors_changed`] — the floor-set edge
/// [`tick_power_system`] forwards from [`PowerSystem::tick`] — so a NEW cut
/// re-announces itself even while the ship has been continuously draining.
///
/// # `sender_origin`
///
/// Resolved from the ship's own `power-reactor` control source — the allocation
/// surface, i.e. the system whose state *is* the brownout, and the same
/// representative `handle_power_messages` admits against. Until issue #873 this
/// was hardcoded to `ControlSource::Ai`, which is a routing-tag lie in the
/// opposite direction from the emit-side branches that issue removed: a
/// human-operated Power console's advisory claimed AI origin, so
/// `route_coordination` raised a popup at a human Helm/Tactical where two humans
/// on the same bridge should simply talk to each other (Suppress). Nothing here
/// branches on the value — it is stamped and forgotten, exactly as
/// `detect_damage_tier_crossings` stamps its own.
pub fn tick_power_brownout_advisory(
    mut ships: Query<
        (
            Entity,
            &ShipPowerSystem,
            &mut PowerBrownoutState,
            Option<&PowerConfigResource>,
            &crate::ship_plugin::ShipSystemControlSources,
        ),
        With<crate::server_app::Ship>,
    >,
    config_res: Option<Res<PowerConfigResource>>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, power, mut brownout_state, config_comp, control_sources) in ships.iter_mut() {
        let sender_origin = control_sources
            .0
            .source_for(&crate::system_registry::power_reactor_system_id());
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
        let is_draining = power.0.is_draining(&cfg.0);

        // Consume this tick's floor-set edge. A group the reactor has just cut
        // (or just released) is a genuine transition, so clear the debounce and
        // let the groups still in brownout re-announce below.
        if std::mem::take(&mut brownout_state.floors_changed) {
            brownout_state.notified_groups.clear();
        }

        let mut still_brownouting = std::collections::HashSet::new();

        for (group_id, level) in power.0.iter() {
            if (is_draining || power.0.is_floored(group_id)) && level > 1 {
                still_brownouting.insert(group_id.0.clone());

                // Rising edge: group was not previously notified → emit advisory.
                if !brownout_state.notified_groups.contains(&group_id.0) {
                    if let Some(sys_id) = system_id_for_power_group(&group_id.0) {
                        writer.write(CoordinationEnqueue {
                            source_entity: entity,
                            sender_origin,
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
        .map(|cs| !cs.0.is_offline(&reactor_id))
        .unwrap_or(true);
    let battery_online = control_sources
        .map(|cs| !cs.0.is_offline(&battery_id))
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
                // The standing order, so the panel's +/- can step from what was
                // ASKED for rather than from what a battery floor has left the
                // group running at (issue #952).
                commanded_level: power.0.commanded_level_for(&gid),
                max_level,
            }
        })
        .collect();

    let bb = PowerBlackboard {
        groups: entries,
        total: power.0.total(),
        total_max: crate::modifiers::power_system::MAX_COMMANDED_TOTAL,
        battery_charge: power.0.battery_charge,
        battery_max: config.0.capacity,
        draining: power.0.is_draining(&config.0),
        charging: power.0.is_charging(&config.0),
    };
    // Fine blackboards (issue #513) — reactor owns the allocation surface,
    // battery owns the emergency-reserve pool. Emitted alongside the legacy
    // coarse `Power` blackboard so downstream JS panels can pick either or both.
    let reactor_bb = PowerReactorBlackboard {
        total_allocation: power.0.total(),
        max_allocation: crate::modifiers::power_system::MAX_COMMANDED_TOTAL,
        is_online: reactor_online,
        draining: power.0.is_draining(&config.0),
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
    use crate::power_system::SHIELDS_POWER_GROUP;
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
            // Chain the SimSet phases so admission (before Input) → the applier
            // `handle_power_messages` (moved to Physics in issue #831) → battery
            // tick → publish run in order. Without this, `handle_power_messages`
            // in Physics has no ordering vs. the `.before(Input)` AdmissionSet,
            // so it can run before the command is admitted and the allocation
            // never applies (mirrors the navigation test harness's #830 chain).
            // In `FixedUpdate`, where `ShipPowerPlugin` and `AdmissionPlugin`
            // register since issue #895 — configured on `Update` this chain
            // would order nothing at all.
            .configure_sets(
                FixedUpdate,
                (
                    crate::sim_sets::SimSet::Input,
                    crate::sim_sets::SimSet::Physics,
                    crate::sim_sets::SimSet::Damage,
                    crate::sim_sets::SimSet::Modifiers,
                    crate::sim_sets::SimSet::Publish,
                    crate::sim_sets::SimSet::PublishAggregate,
                    crate::sim_sets::SimSet::Broadcast,
                )
                    .chain(),
            )
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .add_plugins(ShipPowerPlugin)
            .add_systems(
                FixedUpdate,
                crate::modifier_coordination::translate_power_modifiers.after(tick_power_system),
            )
            .add_plugins(crate::simulation::sim_state_broadcaster())
            .add_systems(PostUpdate, collect);
        // Exactly one fixed step per `update()` (issue #895), advancing 200 ms
        // of sim time so the Hz-based broadcast timers always fire inside a
        // single harness tick — the pace this fixture has always run at, which
        // a bare `ManualDuration` no longer delivers now the sim is in
        // `FixedUpdate` against the default 60 Hz timestep.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
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

    // ── Battery floors (issue #952) ─────────────────────────────────────────

    /// `authored_power_group_floors` pairs each authored percentage with its
    /// group's OWN `[power_groups.<id>] min_level`, and falls back to 1 for a
    /// group the hull does not describe (the NPC case, where the runtime seeds
    /// the canonical trio and no `[power_groups.*]` block exists).
    #[test]
    fn authored_floors_take_their_landing_level_from_the_groups_own_min_level() {
        use crate::ship::config::PowerGroupConfig;
        let groups = std::collections::HashMap::from([(
            PowerGroupId(HELM_POWER_GROUP.into()),
            PowerGroupConfig {
                label: "helm".into(),
                default_level: 2,
                min_level: 2,
                max_level: 4,
            },
        )]);
        let pct = std::collections::HashMap::from([
            (HELM_POWER_GROUP.to_string(), 50.0),
            (SHIELDS_POWER_GROUP.to_string(), 5.0),
        ]);

        let floors = authored_power_group_floors(&pct, &groups);
        assert_eq!(floors[HELM_POWER_GROUP].battery_pct, 50.0);
        assert_eq!(
            floors[HELM_POWER_GROUP].min_level, 2,
            "a brownout must not push a group under the floor its own file sets"
        );
        assert_eq!(
            floors[SHIELDS_POWER_GROUP].min_level,
            crate::modifiers::power_system::UNAUTHORED_FLOOR_LEVEL,
            "a group the hull does not describe lands on the level the runtime              seeds it at, not on an invented 1"
        );
    }

    /// **Every shipped hull ships the ladder, it is a LADDER, and it lands each
    /// group where that group's own file says it may sit.**
    ///
    /// Walked on the shipped files through the include resolver rather than
    /// asserted on the parse default, because the whole point of issue #952's
    /// AC4 is that these are authored per hull.
    ///
    /// # What this used to also assert, and why it no longer does
    ///
    /// An earlier revision required each floor to sit STRICTLY BELOW the same
    /// group's `[power.ai_policy.param] min_reserve_<g>`, on the reading that
    /// the two answer the same question through different mechanisms and would
    /// race when authored at the same percentage. They did race — but the cause
    /// was that neither had hysteresis, and separating them by ten points hid
    /// the collision at the cost of the feature.
    ///
    /// The cost was total, not partial. With the floor under the reserve, the
    /// policy has already commanded the group down to the floor's own landing
    /// level (`min_level`) before the charge reaches the floor, so
    /// `apply_battery_floors`' `held >= commanded` skip fires and the group is
    /// never cut. On every AI-crewed hull in the fleet — which is every hull
    /// but the one a human is sitting at — the ladder did nothing at all, and
    /// "shields hold longest" was true only in the sense that nothing was ever
    /// cut. An invariant that holds only because the feature it guards is inert
    /// is not worth keeping.
    ///
    /// [`crate::modifiers::power_system::PowerConfig::floor_release_margin_pct`]
    /// is what makes the relation safe to break: the floor releases a margin
    /// ABOVE its own threshold, so a floor authored at or above a reserve damps
    /// the policy's boundary instead of racing it. What survives is a per-group
    /// TUNING choice rather than a universal relation, and the fleet answers it
    /// two different ways on purpose — `weapons` above its reserve so the
    /// reactor cuts the guns before the crew would, `helm` below its reserve
    /// because holding helm down through a band the crew has already
    /// re-authorised measurably costs an attack-pass destroyer its break-off.
    /// `console_ai::server::tests::the_battery_floor_ladder_cuts_an_ai_crewed_hull_in_floor_order`
    /// pins the effect, and the `[power.battery_floor]` note in
    /// `assets/entities/fragments/ai/fleet_baseline.toml` records the
    /// measurement. What IS still checked below is the release margin, and
    /// against the hazard it actually guards rather than against the reserve.
    #[test]
    fn every_hulls_battery_floors_descend() {
        let mut checked: Vec<String> = Vec::new();
        for path in shipped_entity_paths() {
            let config = crate::entity_includes::load_entity_config(&path)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let Some(reactor) = config.power.as_ref() else {
                continue;
            };
            let power_groups = config
                .ship_config
                .as_ref()
                .map(|s| s.power_groups.clone())
                .unwrap_or_default();
            let floors = authored_power_group_floors(&reactor.battery_floor, &power_groups);

            let read = |g: &str| {
                floors
                    .get(g)
                    .unwrap_or_else(|| panic!("{path} authors no `{g}` battery floor"))
                    .battery_pct
            };
            let (helm, weapons, shields) = (
                read(HELM_POWER_GROUP),
                read(WEAPONS_POWER_GROUP),
                read(SHIELDS_POWER_GROUP),
            );
            assert!(
                helm > weapons && weapons > shields,
                "{path}: floors are helm {helm} / weapons {weapons} / shields \
                 {shields}. They must DESCEND in that order or the ship does not \
                 lose helm first and keep its screens longest, which is the entire \
                 behaviour #952 buys"
            );

            // A floor that can BITE needs the hysteresis band, full stop.
            //
            // This used to read `floor < reserve || margin > 0.0`, which guards
            // the wrong hazard: it let the margin be switched off on any floor
            // authored under the same group's `min_reserve_*` — which is the
            // shipped `helm = 40` against a `min_reserve_helm` of 50, on the
            // very hull a human Power officer flies. The AI reserve has nothing
            // to do with the flicker. At margin 0 the cut and release
            // thresholds are the same number, so the group is released the tick
            // after it is cut and chatters at tick rate, and that happens
            // whenever the group can be COMMANDED above the level its floor
            // lands it on, by any operator.
            //
            // Also checked at load in `EntityConfig::from_toml`, so a hull
            // added outside `assets/entities` cannot get past it either; this
            // walk keeps the statement where the rest of the ladder's fleet
            // invariants live.
            let margin = reactor.battery_floor_release_margin;
            for (group, _) in floors.iter() {
                let can_bite = match power_groups.get(&PowerGroupId(group.clone())) {
                    Some(cfg) => cfg.min_level < cfg.max_level,
                    None => {
                        crate::modifiers::power_system::UNAUTHORED_FLOOR_LEVEL
                            < crate::ship::config::default_max_power_level()
                    }
                };
                assert!(
                    !can_bite || margin > 0.0,
                    "{path}: `[power.battery_floor] {group}` can bite — the group can be \
                     commanded above the level its floor lands it on — but \
                     `battery_floor_release_margin` is {margin}. With no band the cut \
                     and its release share one threshold and flip at tick rate"
                );
            }

            // Every group the hull authors and the ladder names must land on
            // that group's own min_level — the two blocks have to agree.
            for (id, cfg) in &power_groups {
                if let Some(floor) = floors.get(id.0.as_str()) {
                    assert_eq!(
                        floor.min_level, cfg.min_level,
                        "{path}: `{}`'s floor lands on {} but its `[power_groups]` \
                         min_level is {}",
                        id.0, floor.min_level, cfg.min_level
                    );
                }
            }
            checked.push(path);
        }
        assert!(
            checked.len() >= 9,
            "only {} shipped hull(s) reached this invariant ({checked:?}). Every \
             ship in the fleet authors a `[power]` block; if fewer than nine were \
             walked, the walk stopped finding them rather than the fleet having \
             got smaller. It was ten until #954 moved the three-weapon RNG-coverage \
             escort out of `assets/entities/` to the test fixture directory — a hull \
             leaving the fleet is the one reason this number may fall",
            checked.len()
        );
    }

    /// **A fully-floored hull must still RECHARGE.**
    ///
    /// The trap the `alliance_courier` shipped, and the one no other test could
    /// see. `PowerSystem::tick` integrates the battery from the EFFECTIVE
    /// total, so once every floor is in force the ship's draw is the sum of
    /// each group's landing level — and if the hull's own `[power] rates`
    /// happen to put a zero (or worse) on that rung, the reserve stops moving,
    /// no floor can ever climb back through its release margin, and the ship is
    /// parked at its floors for the rest of the encounter with no way out.
    /// The courier's `rates` put exactly `0.0` at its floored total of 6
    /// (`ops 1 + helm 2 + weapons 2 + shields 1`), which also meant a courier
    /// sitting at anchor never recovered a point of charge.
    ///
    /// The retired brownout lock hid this by forcing every group to 1, dropping
    /// the total to the reactor's fastest-charging rung whatever the hull had
    /// authored. Per-group floors land where the hull says, so the hull has to
    /// mean it.
    ///
    /// # Computing the total the way `tick` does
    ///
    /// The draw modelled here has to be the WORST the runtime can land on, not
    /// the one a resting hull happens to show, and two things nearly made it
    /// the latter:
    ///
    /// * The landing level is `min_level` CLAMPED to `[1, 4]`, because
    ///   `apply_battery_floors` clamps it. `PowerGroupConfig::min_level` has no
    ///   load-time range check, so an authored `min_level = 0` would otherwise
    ///   make this walk compute a total the ship can never actually draw and
    ///   clear the assertion on a rung it never lands on.
    /// * A floored group lands on that level whatever it was commanded to, so
    ///   the resting level is the wrong input for it — a hull authoring
    ///   `default_level` under `min_level` would be modelled low. Only a group
    ///   the ladder does NOT name keeps its commanded level, and for those this
    ///   walk models the resting level. That is honest only while no such group
    ///   can be commanded at all, which is asserted below rather than assumed:
    ///   `ops` on the four Alliance hulls is not in `POWER_GROUP_ORDER`, so
    ///   `publish_power_blackboard` never puts it on the panel, and no shipped
    ///   `[power.ai_policy]` names it as a channel. Were it commandable, an
    ///   `ops 3 / helm 2 / weapons 2 / shields 1 = 8` would be a fully-floored
    ///   destroyer sitting on `rates[5] = -5` — the exact permanent park this
    ///   test exists to prevent, invisible to it.
    #[test]
    fn every_hulls_fully_floored_total_recharges() {
        let mut checked: Vec<String> = Vec::new();
        for path in shipped_entity_paths() {
            let config = crate::entity_includes::load_entity_config(&path)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let Some(reactor) = config.power.as_ref() else {
                continue;
            };
            let power_groups = config
                .ship_config
                .as_ref()
                .map(|s| s.power_groups.clone())
                .unwrap_or_default();
            let floors = authored_power_group_floors(&reactor.battery_floor, &power_groups);

            // The allocation a fully-floored ship settles on: every group the
            // ladder names at its landing level, every group it does not at the
            // level its file rests it on. Where the hull authors no
            // `[power_groups.*]` at all, the runtime seeds the canonical trio,
            // which `authored_power_group_floors` has already accounted for.
            let seed = authored_power_group_seed(&power_groups);
            let resting: Vec<(String, u8)> = if seed.is_empty() {
                POWER_GROUP_ORDER
                    .iter()
                    .map(|g| {
                        (
                            (*g).to_string(),
                            crate::modifiers::power_system::UNAUTHORED_FLOOR_LEVEL,
                        )
                    })
                    .collect()
            } else {
                seed.iter().map(|(id, lvl)| (id.0.clone(), *lvl)).collect()
            };
            // A group the ladder names lands on its clamped `min_level` however
            // it was commanded; a group it does not keeps its commanded level,
            // which is its resting level for as long as nothing can command it.
            let ai_channels: Vec<&str> = reactor
                .ai_policy
                .as_ref()
                .map(|p| {
                    p.rule
                        .iter()
                        .chain(p.state.iter().flat_map(|s| s.rule.iter()))
                        .map(|r| r.channel.as_str())
                        .collect()
                })
                .unwrap_or_default();
            let floored_total: u32 = resting
                .iter()
                .map(|(id, resting_level)| match floors.get(id.as_str()) {
                    Some(f) => f.min_level.clamp(1, 4) as u32,
                    None => {
                        assert!(
                            !POWER_GROUP_ORDER.contains(&id.as_str())
                                && !ai_channels.contains(&id.as_str()),
                            "{path}: `{id}` has no `[power.battery_floor]` rung but CAN be \
                             commanded — it is on the Power panel or is an authored \
                             `[power.ai_policy]` channel. The ladder cannot bound a group \
                             it does not name, so the total below is not the worst this \
                             hull can draw and this whole invariant is unenforceable. \
                             Author a floor for it"
                        );
                        *resting_level as u32
                    }
                })
                .sum();
            let rung = floored_total.clamp(3, 8) as usize - 3;
            let rate = reactor.rates[rung];
            assert!(
                rate > 0.0,
                "{path}: fully floored this hull draws {floored_total}, and its \
                 `[power] rates` put {rate} on that rung. A ship whose floors settle \
                 it on a non-positive rate can never climb back through a release \
                 margin, so no floor ever lifts and the hull is parked at its \
                 minimums for the rest of the encounter. Author a positive rate at \
                 index {rung}"
            );
            checked.push(path);
        }
        assert!(
            checked.len() >= 9,
            "only {} shipped hull(s) reached this invariant ({checked:?}). Nine, not \
             ten, since #954 moved the RNG-coverage escort out of the fleet",
            checked.len()
        );
    }

    /// Load a shipped hull's reactor as the runtime configures it: capacity,
    /// rates, the authored `[power.battery_floor]` ladder paired with each
    /// group's own `min_level`, and the release margin. Returns the config
    /// alongside a `PowerSystem` seeded at the hull's authored resting levels.
    fn shipped_reactor(path: &str) -> (crate::modifiers::power_system::PowerConfig, PowerSystem) {
        let config = crate::entity_includes::load_entity_config(path)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let reactor = config.power.as_ref().expect("hull authors [power]");
        let power_groups = config
            .ship_config
            .as_ref()
            .map(|s| s.power_groups.clone())
            .unwrap_or_default();
        let cfg = crate::modifiers::power_system::PowerConfig {
            capacity: reactor.capacity,
            rates: reactor.rates,
            emergency_threshold: reactor.emergency_threshold,
            group_floors: authored_power_group_floors(&reactor.battery_floor, &power_groups),
            floor_release_margin_pct: reactor.battery_floor_release_margin,
        };
        let seed = authored_power_group_seed(&power_groups);
        let ps = PowerSystem::from_authored_groups(reactor.capacity, &seed);
        (cfg, ps)
    }

    /// **A hull a HUMAN Power officer has spent up must be able to climb back
    /// out of its own ladder.**
    ///
    /// The failure this pins is not the hysteresis — that part was always
    /// right — but what the hysteresis released INTO. A floored group is
    /// released at `floor + margin`, yet the charge the reactor can actually
    /// reach is set by the LOWEST engaged floor, because that is the cut deep
    /// enough to flip the `rates` rung positive. Release the lowest rung first
    /// and the draw goes straight back up, so the reserve never climbs to the
    /// higher rung's release band at all.
    ///
    /// On the shipped destroyer (capacity 70, `rates = [5,4,3,2,-2,-5]`) with
    /// the legal 8-point order `ops 1 / helm 3 / weapons 3 / shields 1`:
    ///
    /// | charge | engaged | effective total | rate |
    /// |---|---|---|---|
    /// | < 40 % | helm | 7 | −2/s |
    /// | < 25 % | helm + weapons | 6 | **+2/s** |
    /// | ≥ 30 % | helm only, weapons released at 25+5 | 7 | −2/s |
    ///
    /// +2 and −2 are symmetric, so the charge sat in an exact 25–30 % limit
    /// cycle with zero net drift, and helm's release threshold of 40+5 = 45 %
    /// was unreachable for the rest of the encounter. Nothing on the Power
    /// panel explained it, and the only exit was lowering a DIFFERENT group's
    /// standing order.
    ///
    /// `apply_battery_floors` now releases the ladder from the TOP: a rung
    /// stays cut while any rung above it is still engaged, so the reserve holds
    /// the deep-cut draw all the way up through the highest engaged release
    /// band and every group returns together. This test flies the hull's own
    /// file rather than a fixture, so a retune that re-creates the trap fails
    /// here.
    #[test]
    fn a_human_commanded_destroyer_climbs_back_out_of_its_own_floor_ladder() {
        let path = "assets/entities/alliance_destroyer.toml";
        let (config, mut ps) = shipped_reactor(path);
        let helm = PowerGroupId(HELM_POWER_GROUP.into());
        let weapons = PowerGroupId(WEAPONS_POWER_GROUP.into());

        // The order a human Power officer can set from the console: the whole
        // 8-point budget, combat stations on helm and guns. No AI policy is
        // involved and there is no `min_reserve_*` guard to give a point back.
        ps.set_group_allocation(&helm, 3).unwrap();
        ps.set_group_allocation(&weapons, 3).unwrap();
        assert_eq!(
            ps.commanded_total(),
            8,
            "the officer spends the full budget"
        );

        let pct = |ps: &PowerSystem| ps.battery_charge / config.capacity * 100.0;
        let helm_release =
            config.group_floors[HELM_POWER_GROUP].battery_pct + config.floor_release_margin_pct;

        let dt = 0.1_f32;
        let mut helm_was_cut = false;
        let mut high_water = 0.0_f32;
        let mut recovered_after = None;
        // Two minutes of simulated time — the whole drain-and-recover excursion
        // above is about twenty seconds, so a run that does not recover here is
        // not recovering at all.
        for step in 0..1200 {
            ps.tick(dt, &config);
            if ps.is_floored(&helm) {
                helm_was_cut = true;
            }
            if !helm_was_cut {
                continue;
            }
            high_water = high_water.max(pct(&ps));
            if !ps.is_floored(&helm) && !ps.is_floored(&weapons) && pct(&ps) >= helm_release {
                recovered_after = Some(step as f32 * dt);
                break;
            }
        }

        assert!(
            helm_was_cut,
            "{path}: the reserve never fell far enough to cut helm, so this test \
             is not exercising the ladder at all"
        );
        let elapsed = recovered_after.unwrap_or_else(|| {
            panic!(
                "{path}: helm was cut and never came back. The charge peaked at \
                 {high_water:.1} % against a helm release threshold of \
                 {helm_release:.1} % — the ladder released its LOWEST engaged \
                 rung first, which put the draw back up and capped the reserve \
                 below every higher rung's release band. A rung must stay cut \
                 while any rung above it is still engaged"
            )
        });
        assert!(
            !ps.is_floored(&helm) && !ps.is_floored(&weapons),
            "every group must be back at its standing order once the reserve has \
             cleared the top of the ladder"
        );
        assert_eq!(
            ps.total(),
            ps.commanded_total(),
            "recovery is free: the effective total returns to what the officer \
             commanded without anyone re-issuing anything (after {elapsed:.1} s)"
        );
    }

    /// Every shipped `assets/entities/*.toml`, as the relative paths the include
    /// resolver keys on. Read off the directory rather than listed, so a new hull
    /// is covered by the invariant above the moment it is added.
    fn shipped_entity_paths() -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir("assets/entities")
            .expect("assets/entities must be readable")
            .map(|e| e.expect("readable dir entry").path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        out.sort();
        out
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
                &crate::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
                2,
            );
        }
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.increase(&PowerGroupId(SHIELDS_POWER_GROUP.into()));
        }
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
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

    /// **A flat battery drives every group to its floor, and the floors reach
    /// the modifier table.**
    ///
    /// Renamed and re-aimed from `exhaustion_forces_consoles_to_one_and_updates_all_modifiers`
    /// (issue #952), which asserted that a flat battery slammed every group to
    /// 1 and crushed all three multipliers to x0.667. Both halves of that are
    /// now wrong. Nothing "forces consoles to one": each group is held at its
    /// own authored floor level, which for a hull that describes no
    /// `[power_groups.*]` — this fixture — is NOMINAL. So the assertion
    /// inverts: the spent-up helm loses its point and lands on x1.0, and the
    /// two groups that were resting there never move at all. The third slot
    /// changed too — `RadarRange` was the sensors group's and is now nobody's,
    /// so the group whose collapse shows in the modifiers is shields, through
    /// `ShieldRegen`.
    #[test]
    fn a_flat_battery_floors_every_group_and_updates_all_modifiers() {
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
            .insert(PowerGroupId(SHIELDS_POWER_GROUP.into()), defaults);

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
                &crate::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
                2,
            );
            ps.0.battery_charge = 0.0;
        }

        tick(&mut app);

        let power = app.world().resource::<ShipPowerSystem>().0.clone();
        assert_eq!(
            power.level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
            2,
            "the flat battery took helm's spent point back"
        );
        assert_eq!(
            power.commanded_level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
            4,
            "and left the command that spent it standing"
        );

        let mods = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipModifiers, With<crate::simulation::LocalShip>>();
            q.single(app.world()).unwrap().clone()
        };

        for (slot, label) in [
            (ModifierSlot::MaxSpeed, "MaxSpeed"),
            (ModifierSlot::PhaserDamage, "PhaserDamage"),
            (ModifierSlot::ShieldRegen, "ShieldRegen"),
        ] {
            let mult = mods.get(&slot);
            assert!(
                (mult - 1.0).abs() < 1e-6,
                "with every group held at its NOMINAL floor, {label} should be                  x1.0, got {mult}. A value below 1 means something landed a group                  under the level its file seeds it at"
            );
        }
        assert!(
            (mods.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6,
            "the reactor must not touch RadarRange at all (#952), got {}",
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
            bb.groups.iter().any(|e| e.label == "SHIELDS"),
            "expected SHIELDS entry"
        );
        assert!(bb.total > 0, "total should be > 0");
        // Total 6 on the default seed, where the default `rates` are still
        // positive — the reserve is filling, not emptying.
        assert!(!bb.draining, "should not be draining at the resting total");
    }

    #[test]
    fn publish_power_blackboard_reflects_helm_level_change() {
        let mut app = test_app();
        // Human holds Power; this test app doesn't register any power AI
        // system anyway (that lives in ConsoleAiPlugin), but a human holder
        // keeps the scenario realistic.
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
                    group: crate::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
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
                    group: crate::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
            4,
            "raw wire string \"power-reactor\" must reach handle_power_messages \
             — if this fails, either the handler's for_target() argument or the \
             JS action-map target has drifted from \"power-reactor\"."
        );
    }

    // ── Inter-system command channel tests (issue #559) ───────────────────────
    //
    // `operate_power_ai`'s tests were removed in issue #693 along with the
    // system itself. System-level coverage for the replacement
    // `ai_power_allocation` (which since issue #831 emits admitted commands
    // applied by `handle_power_messages`) lives in `console_ai::server`'s test
    // module.
    //
    // These tests exercise the full weapons→power inter-system drain flow.
    // A minimal combined app registers both `drain_power_for_active_beam`
    // (Weapons, Physics) and `handle_power_inter_system` (Power, Modifiers)
    // with SimSets chained, so we can set an active beam and verify the
    // Power battery decreases in the same tick.

    fn inter_system_test_app() -> App {
        use crate::console::weapons::{drain_power_for_active_beam, ActiveBeam};
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
        use crate::console::weapons::ActiveBeam;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::simulation::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            // Per-bank since issue #790; this fixture drives the legacy single
            // implicit bank (`""`), which is what a ship with no authored banks
            // fires from.
            match uuid {
                Some(u) => b.start("", u, 1.0),
                None => {
                    b.end_bank("");
                }
            }
        }
    }

    #[test]
    fn active_beam_drains_power_battery_via_inter_system_channel() {
        use crate::console::weapons::PHASER_BATTERY_DRAIN_PER_SEC;
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
        use crate::console::weapons::{drain_power_for_active_beam, ActiveBeam};
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
        use crate::console::weapons::ActiveBeam;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::simulation::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            // Per-bank since issue #790; this fixture drives the legacy single
            // implicit bank (`""`), which is what a ship with no authored banks
            // fires from.
            match uuid {
                Some(u) => b.start("", u, 1.0),
                None => {
                    b.end_bank("");
                }
            }
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
        cs.0.set_offline(system_id, true);
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
            // Chain the SimSet phases so admission (before Input) → the applier
            // `handle_power_messages` (moved to Physics in issue #831) → battery
            // tick → publish run in order. Without this, `handle_power_messages`
            // in Physics has no ordering vs. the `.before(Input)` AdmissionSet,
            // so it can run before the command is admitted and the allocation
            // never applies (mirrors the navigation test harness's #830 chain).
            // In `FixedUpdate` since issue #895 — see `test_app` above.
            .configure_sets(
                FixedUpdate,
                (
                    crate::sim_sets::SimSet::Input,
                    crate::sim_sets::SimSet::Physics,
                    crate::sim_sets::SimSet::Damage,
                    crate::sim_sets::SimSet::Modifiers,
                    crate::sim_sets::SimSet::Publish,
                    crate::sim_sets::SimSet::PublishAggregate,
                    crate::sim_sets::SimSet::Broadcast,
                )
                    .chain(),
            )
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .add_plugins(ShipPowerPlugin)
            .add_plugins(crate::simulation::sim_state_broadcaster());
        // One fixed step per update, 200 ms of sim time each (issue #895).
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
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
                    group: PowerGroupId(SHIELDS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
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
                    group: PowerGroupId(SHIELDS_POWER_GROUP.into()),
                    level: 1,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
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
            cs.0.set_offline(crate::system_registry::power_reactor_system_id(), true);
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
            cs.0.set_offline(crate::system_registry::power_battery_system_id(), true);
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
            cs.0.set_offline(crate::system_registry::power_reactor_system_id(), true);
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
        set_ship_power(&mut app, SHIELDS_POWER_GROUP, 1);
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
                        SHIELDS_POWER_GROUP,
                        "sensors at level 1 must not get a brownout advisory"
                    );
                }
                _ => panic!("unexpected payload type"),
            }
        }
    }

    /// **A group crossing its floor must not re-fire the advisory at tick rate.**
    ///
    /// The regression this pins is the one the release margin exists for, seen
    /// from the far end of the bus. Flooring a group lowers the EFFECTIVE total,
    /// `PowerSystem::tick` indexes `rates` by the effective total, and the rungs
    /// either side of the resting total have opposite signs — so with a single
    /// threshold the cut recharges the reserve straight back through the
    /// threshold that made it, and the group is released on the very next tick.
    /// `is_draining` toggles with it, this system's debounce clears and re-arms
    /// on alternating ticks, and `CoordinationPayload::PowerBrownout` lands at
    /// Helm and Tactical around thirty times a second.
    ///
    /// So this counts TICKS THAT EMITTED rather than asserting a state: a
    /// message count is the thing a bridge crew actually experiences, and it is
    /// the only assertion that fails loudly if the two-threshold shape is ever
    /// flattened back into one.
    #[test]
    fn a_group_crossing_its_floor_does_not_re_advise_every_tick() {
        let mut app = brownout_test_app();
        start_game(&mut app);

        // Commanded total 7 → −2/s on the default rates; the floored total of 6
        // is +2/s, so the cut flips the sign. Start just above helm's 50 % floor.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPowerSystem, With<crate::simulation::LocalShip>>();
            if let Ok(mut ps) = q.single_mut(app.world_mut()) {
                let _ =
                    ps.0.set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 3);
                ps.0.battery_charge = 50.5;
            }
        }
        let _ = tick(&mut app);
        let _ = drain_coord(&mut app);

        let mut emitting_ticks = 0;
        let mut helm_flips = 0;
        let mut last_helm = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipPowerSystem, With<crate::simulation::LocalShip>>();
            q.single(app.world())
                .unwrap()
                .0
                .level_for(&PowerGroupId(HELM_POWER_GROUP.into()))
        };
        const TICKS: usize = 40;
        for _ in 0..TICKS {
            let _ = tick(&mut app);
            if !drain_coord(&mut app).is_empty() {
                emitting_ticks += 1;
            }
            let helm = {
                let mut q = app
                    .world_mut()
                    .query_filtered::<&ShipPowerSystem, With<crate::simulation::LocalShip>>();
                q.single(app.world())
                    .unwrap()
                    .0
                    .level_for(&PowerGroupId(HELM_POWER_GROUP.into()))
            };
            if helm != last_helm {
                helm_flips += 1;
                last_helm = helm;
            }
        }

        assert!(
            emitting_ticks <= 4,
            "PowerBrownout advisories went out on {emitting_ticks} of {TICKS} ticks. \
             A brownout that re-announces itself every other tick is the \
             single-threshold flip-flop: the cut drops the draw onto a charging \
             rung, the charge re-crosses the bare floor, and the group is released \
             again immediately"
        );
        assert!(
            helm_flips <= 4,
            "helm's effective level changed {helm_flips} times in {TICKS} ticks — \
             the floor is chattering, and every consumer of `MaxSpeed` is \
             chattering with it"
        );
        assert!(
            emitting_ticks >= 1,
            "the fixture never browned out at all, so it proves nothing: helm \
             must actually cross its floor inside the window"
        );
    }

    /// Issue #873. The brownout advisory's `sender_origin` must report the
    /// Power console's LIVE control source, not a hardcoded `ControlSource::Ai`.
    ///
    /// The hardcode was a routing-tag lie in the opposite direction from the
    /// emit-side branches #873 removed: `route_coordination` reads the tag to
    /// pick Consume / Popup / Suppress, so a human-operated Power console's
    /// advisory claimed AI origin and raised a popup at a human Helm or
    /// Tactical, where two humans on the same bridge should simply talk
    /// (Suppress). The tag is stamped and forgotten — it is checked here at the
    /// point of emission precisely because nothing downstream may re-derive it.
    #[test]
    fn brownout_advisory_tags_the_live_power_control_source() {
        use crate::control_source::ControlSource;
        for source in [ControlSource::Human, ControlSource::Ai] {
            let mut app = brownout_test_app();
            start_game(&mut app);
            {
                let mut q = app
                    .world_mut()
                    .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::simulation::Ship>>();
                for mut cs in q.iter_mut(app.world_mut()) {
                    cs.0.set(crate::system_registry::power_reactor_system_id(), source);
                }
            }
            let _ = tick(&mut app);
            let _ = drain_coord(&mut app);

            // total=7 → draining → every group above idle emits.
            {
                let mut q = app
                    .world_mut()
                    .query_filtered::<&mut ShipPowerSystem, With<crate::simulation::LocalShip>>();
                if let Ok(mut ps) = q.single_mut(app.world_mut()) {
                    let _ =
                        ps.0.set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 3);
                }
            }
            let _ = tick(&mut app);
            let emitted = drain_coord(&mut app);
            assert!(
                !emitted.is_empty(),
                "fixture must actually produce a brownout advisory for {source:?}"
            );
            for e in &emitted {
                assert_eq!(
                    e.sender_origin, source,
                    "PowerBrownout must carry the reactor's live control source as its \
                     routing tag; a hardcoded origin sends a human Power officer's \
                     advisory down the AI→human popup path"
                );
            }
        }
    }
}

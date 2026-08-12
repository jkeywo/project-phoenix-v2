use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{
    CoordinationPayload, PowerBatteryBlackboard, PowerBlackboard, PowerGroupEntry, PowerGroupId,
    PowerReactorBlackboard, ServerMessage, SystemBlackboard, SystemId,
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
    /// the reactor's exhaustion lock changed state this tick, and consumed (and
    /// cleared) by [`tick_power_brownout_advisory`] later in the same tick.
    ///
    /// This is the edge-triggered half of the advisory (issue #678). The
    /// debounce below is level-triggered off the reserve's direction; the edge
    /// re-arms it the tick the reactor locks out so the brownout announces
    /// itself to Helm or Tactical.
    pub locked_changed: bool,
}

/// Maps a canonical power group id string to the `strings.csv` id for its
/// display label. Anything unknown falls back to `power.group.unknown`.
///
/// **This function is the live path, for every hull.** It is the sole producer
/// of [`PowerGroupEntry::label`] and of [`CoordinationPayload::PowerBrownout`]'s
/// label, and nothing in `gui/` reads a `power_groups` block. Per issue #977 it
/// emits a `strings.csv` id, never composed English: the blackboard and the
/// coordination payload both cross `localiseTree` at the wire boundary, which
/// resolves the id to "HELM" / "WEAPONS" / "SHIELDS" client-side. The authored
/// `[power_groups.<id>] label` fields are real ids too and covered by
/// `scripts/check-strings.mjs`; wiring them up means carrying the ship's
/// authored labels into `publish_power_blackboard`, with these as the fallback
/// for a hull that authors none.
pub fn power_group_label(group_id: &str) -> &'static str {
    match group_id {
        HELM_POWER_GROUP => "power.group.helm",
        WEAPONS_POWER_GROUP => "power.group.weapons",
        SHIELDS_POWER_GROUP => "power.group.shields",
        _ => "power.group.unknown",
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

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipPowerPlugin;

impl Plugin for ShipPowerPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_power_messages` reads
        // the `power-reactor` system's admitted commands.
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
                locked: power.0.locked(),
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
                    warn!("power.allocation_ignored={err:?}");
                }
            } else if is_local {
                if let Some(pr) = power_res.as_deref_mut() {
                    if let Err(err) = pr.0.set_group_allocation(&group, level) {
                        warn!("power.allocation_ignored={err:?}");
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

/// Tick the power system battery charge each frame.
///
/// After PR 6 (PRD #597): iterates ALL ship entities with a `ShipPowerSystem`
/// component so NPC ships tick their own power. Uses the per-entity
/// `PowerConfigResource` component when present, else the global Resource
/// fallback, so NPC ships without a `[power]` block still tick with defaults.
/// The Resource fallback path is retained for test environments that only
/// insert the resource without a ship entity.
///
/// Forwards [`PowerSystem::tick`]'s lock-changed edge into
/// [`PowerBrownoutState::locked_changed`] on the same ship, for
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
        let locked_changed = power.0.tick(dt, &cfg.0);
        if let Some(mut state) = brownout_state {
            if locked_changed {
                state.locked_changed = true;
            }
        }
        ticked_any = true;
    }
    // Fallback: no ship entity with the component (test environments that only
    // insert the Resource form). Tick the Resource directly. The lock edge is
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
///   (`PowerSystem::is_draining`)
/// - The group's allocation level > 1 (system is actively drawing, not idle)
///
/// This warns Helm or Tactical while the reserve is still emptying — before the
/// reactor bottoms out and locks every group to 1. Once locked there is nothing
/// drawing the extra, so the advisory falls silent; the lock transition itself
/// re-arms the debounce (below) so a fresh drain after recovery re-announces.
///
/// Debounced via [`PowerBrownoutState`]: fires once on transition into
/// brownout and clears when the condition resolves, allowing re-fire. The
/// debounce is additionally re-armed by
/// [`PowerBrownoutState::locked_changed`] — the lock-changed edge
/// [`tick_power_system`] forwards from [`PowerSystem::tick`].
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

        // Consume this tick's lock-changed edge. A reactor that has just locked
        // out (or just recovered) is a genuine transition, so clear the debounce
        // and let the groups still in brownout re-announce below.
        if std::mem::take(&mut brownout_state.locked_changed) {
            brownout_state.notified_groups.clear();
        }

        let mut still_brownouting = std::collections::HashSet::new();

        for (group_id, level) in power.0.iter() {
            if is_draining && level > 1 {
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
                            sender_label: crate::ship::coordination::CHATTER_SENDER_POWER
                                .to_string(),
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
        total_max: power.0.max_commanded_total(),
        battery_charge: power.0.battery_charge,
        battery_max: config.0.capacity,
        draining: power.0.is_draining(&config.0),
        charging: power.0.is_charging(&config.0),
        locked: power.0.locked(),
    };
    // Fine blackboards (issue #513) — reactor owns the allocation surface,
    // battery owns the emergency-reserve pool. Emitted alongside the legacy
    // coarse `Power` blackboard so downstream JS panels can pick either or both.
    let reactor_bb = PowerReactorBlackboard {
        total_allocation: power.0.total(),
        max_allocation: power.0.max_commanded_total(),
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

    /// Issue #977: the label producer emits `strings.csv` ids, never composed
    /// English. Every id it can return must exist in the table (so `localiseTree`
    /// resolves it), and the shape must be a dotted lowercase id.
    #[test]
    fn power_group_label_emits_string_ids_not_english() {
        use crate::power_system::{HELM_POWER_GROUP, WEAPONS_POWER_GROUP};
        for group in [
            HELM_POWER_GROUP,
            WEAPONS_POWER_GROUP,
            SHIELDS_POWER_GROUP,
            "ops",
        ] {
            let id = power_group_label(group);
            assert!(
                id.starts_with("power.group."),
                "{group} → {id:?} must be a power.group.* id, not English"
            );
            assert!(
                !id.chars().any(|c| c.is_ascii_uppercase() || c == ' '),
                "{id:?} must be a dotted lowercase id"
            );
        }
        assert_eq!(power_group_label("ops"), "power.group.unknown");
    }

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

    /// **A flat battery locks the reactor: every group is slammed to 1 and the
    /// collapse reaches the modifier table.**
    ///
    /// The exhaustion lock, restored after issue #952's per-group floors were
    /// reverted — a player who fails to manage power loses the lot, not just the
    /// point they spent up. All three groups land on level 1, so every
    /// multiplier crushes to x0.667. `RadarRange` was the retired sensors
    /// group's and is now nobody's, so the reactor never touches it.
    #[test]
    fn a_flat_battery_locks_every_group_to_one_and_updates_all_modifiers() {
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
        assert!(power.locked(), "a flat battery locks the reactor");
        assert_eq!(
            power.level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
            1,
            "the brownout slammed helm to 1"
        );

        let expected = 1.0 / 1.5;
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
                (mult - expected).abs() < 1e-6,
                "with every group locked to 1, {label} should be x{expected}, got {mult}"
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
        // Labels are `strings.csv` ids now (issue #977); `localiseTree`
        // resolves them to HELM / WEAPONS / SHIELDS at the client boundary.
        assert!(
            bb.groups.iter().any(|e| e.label == "power.group.helm"),
            "expected helm entry"
        );
        assert!(
            bb.groups.iter().any(|e| e.label == "power.group.weapons"),
            "expected weapons entry"
        );
        assert!(
            bb.groups.iter().any(|e| e.label == "power.group.shields"),
            "expected shields entry"
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
        let helm_entry = bb
            .groups
            .iter()
            .find(|e| e.label == "power.group.helm")
            .unwrap();
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

    // ── Fine Power system tests (issue #513) ───────────────────────────────────
    //
    // Cover the reactor offline gate and the reactor/battery blackboard
    // publication.

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

use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::core::messages::{
    CoordinationPayload, PowerBatteryBlackboard, PowerBlackboard, PowerGroupEntry, PowerGroupId,
    PowerReactorBlackboard, ServerMessage, SystemBlackboard, SystemId,
};
use crate::modifiers::power_system::{
    power_level_for_group, AuthoredPowerGroup, PendingGroupPower, PowerConfig, PowerSystem,
    ScriptedPowerLevel, HELM_POWER_GROUP, POWER_GROUP_ORDER, SHIELDS_POWER_GROUP,
    WEAPONS_POWER_GROUP,
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
    use crate::entities::ai_flag_hosts as fid;
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
    facts.set_fact(fid::TOTAL_ALLOCATION, power.total() as f64);
    // THREAT: only set when known, so an absent-fact guard reads "no threat".
    if let Some(s) = secs_since_combat {
        facts.set_fact(fid::SECS_SINCE_COMBAT, s as f64);
    }
    if let Some(d) = nearest_enemy_dist {
        facts.set_fact(fid::NEAREST_ENEMY_DIST, d as f64);
    }
    // OBJECTIVE + SYSTEM.
    facts.set_fact(
        fid::HAS_DESTROY_OBJECTIVE,
        if has_destroy_objective { 1.0 } else { 0.0 },
    );
    facts.set_fact(fid::OFFLINE_SYSTEM_COUNT, offline_system_count as f64);
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

impl PowerBrownoutState {
    /// Read the authoritative advisory debounce for continuation projection.
    pub(crate) fn continuation(&self) -> (&std::collections::HashSet<String>, bool) {
        (&self.notified_groups, self.locked_changed)
    }

    /// Replace bootstrap debounce state with a restored continuation.
    pub(crate) fn replace_continuation(
        &mut self,
        notified_groups: std::collections::HashSet<String>,
        locked_changed: bool,
    ) {
        self.notified_groups = notified_groups;
        self.locked_changed = locked_changed;
    }
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
/// sorted by id, each seeded at its authored `default_level` AND its authored
/// `min_level` (issue #1395 — the floor the reactor stores and clamps every
/// later order to, so a hull that authors `min_level = 0` really can have that
/// group commanded cold). Returns an empty vec when there are no authored
/// groups so the caller falls back to the canonical default seeding (unchanged
/// behaviour for NPCs / fixtures without a `[power_groups.*]` block).
pub fn authored_power_group_seed(
    power_groups: &std::collections::HashMap<PowerGroupId, crate::ship::config::PowerGroupConfig>,
) -> Vec<AuthoredPowerGroup> {
    if power_groups.is_empty() {
        return Vec::new();
    }
    fn entry(id: PowerGroupId, cfg: &crate::ship::config::PowerGroupConfig) -> AuthoredPowerGroup {
        AuthoredPowerGroup {
            id,
            level: cfg.default_level,
            floor: cfg.min_level,
        }
    }
    let mut seed: Vec<AuthoredPowerGroup> = Vec::with_capacity(power_groups.len());
    for &name in POWER_GROUP_ORDER {
        let id = PowerGroupId(name.to_string());
        if let Some(cfg) = power_groups.get(&id) {
            seed.push(entry(id, cfg));
        }
    }
    let mut extra: Vec<(&PowerGroupId, &crate::ship::config::PowerGroupConfig)> = power_groups
        .iter()
        .filter(|(id, _)| !POWER_GROUP_ORDER.contains(&id.0.as_str()))
        .collect();
    extra.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (id, cfg) in extra {
        seed.push(entry(id.clone(), cfg));
    }
    seed
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipPowerPlugin;

impl Plugin for ShipPowerPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::DeclareState;
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_power_messages` reads
        // the `power-reactor` system's admitted commands.
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::POWER_REACTOR_KIND,
            crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID,
        ));
        app.init_resource::<crate::core::messages::InterSystemQueue>()
            .add_message::<CoordinationEnqueue>();
        // The scripted power-order queue `drain_scripted_power_orders` drains
        // (issues #1223, #1398), registered and declared at this owning site —
        // it was the captain plugin's `EffectQueue<(String, bool)>` while
        // restraint was a hold. A transient inter-system queue — drained in
        // full every tick, empty at every fold/snapshot boundary — so
        // `ClearedAtFold`.
        app.init_resource::<crate::effect_queue::EffectQueue<PendingGroupPower>>()
            .declare_state::<crate::effect_queue::EffectQueue<PendingGroupPower>>(
                crate::authoritative::StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            );
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
                    // The scenario's half of the restraint lever, and its
                    // mirror (issue #1398). Both in `Modifiers`, chained: a
                    // scripted order lands and is mirrored in the same tick, so
                    // an `on_flag_set` handler chaining off it fires on the next
                    // pipeline pass exactly as an Engineering officer's order
                    // does.
                    (drain_scripted_power_orders, mirror_weapons_cold_flags)
                        .chain()
                        .in_set(crate::sim_sets::SimSet::Modifiers),
                    // Distinct registered keys; tests/publisher_ordering.rs checks
                    // the exact shared access and ordinary/opposed-order outputs.
                    publish_power_blackboard
                        .in_set(crate::sim_sets::SimSet::Publish)
                        .ambiguous_with(crate::ship::shields::publish_shields_blackboard)
                        .ambiguous_with(crate::console::repair::server::publish_repair_blackboard),
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
            &crate::core::messages::AdmittedCommands,
            Option<&mut ShipPowerSystem>,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    power_res: Option<ResMut<ShipPowerSystem>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
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
        for cmd in admitted.for_target(crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID) {
            if let crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group,
                level,
            } = &cmd.payload
            {
                let applied = if let Some(pc) = power_comp.as_deref_mut() {
                    match pc.0.set_group_allocation(group, *level) {
                        Ok(()) => true,
                        Err(err) => {
                            warn!("power.allocation_ignored={err:?}");
                            false
                        }
                    }
                } else if is_local {
                    match power_res.as_deref_mut() {
                        Some(pr) => match pr.0.set_group_allocation(group, *level) {
                            Ok(()) => true,
                            Err(err) => {
                                warn!("power.allocation_ignored={err:?}");
                                false
                            }
                        },
                        None => false,
                    }
                } else {
                    false
                };
                crate::command_admission::finish_admitted_action_feedback(
                    &mut outbound,
                    cmd,
                    if applied {
                        crate::core::messages::ActionFeedbackOutcome::Applied
                    } else {
                        crate::core::messages::ActionFeedbackOutcome::Refused
                    },
                );
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

/// Map a power group to its explicit Coordination destination. Multi-instance
/// Shields has no coarse System, so it resolves the shared `shield_arc` owner.
fn address_for_power_group(
    config: &crate::ship::config::ShipConfig,
    group: &str,
) -> Option<crate::core::messages::CoordinationAddress> {
    match group {
        WEAPONS_POWER_GROUP => crate::ship::coordination::address_for_system(
            config,
            &crate::ship::system_registry::tactical_radar_system_id(),
        ),
        HELM_POWER_GROUP => crate::ship::coordination::address_for_system(
            config,
            &crate::ship::system_registry::helm_steering_system_id(),
        ),
        SHIELDS_POWER_GROUP => crate::ship::coordination::address_for_system_kind(
            config,
            crate::ship::system_registry::SHIELD_ARC_KIND,
        ),
        _ => None,
    }
}

/// Emit power brownout coordination advisories when the reactor EXHAUSTS —
/// battery charge hits zero and [`PowerSystem::tick`] takes every group DOWN to
/// [`GROUP_LEVEL_MIN`] and locks the reactor.
///
/// This is the brownout, and the only one: the ship actually lost power and
/// every system that was running reset to 1. (A group somebody had already
/// switched off is left cold rather than raised — see `PowerSystem::tick`.) It is emphatically NOT a "reserve running low"
/// warning — a draining-but-managed reactor (the AI shed ladder doing its job
/// on the way down, which is the normal state of any ship holding elevated
/// power in combat) is expected and says nothing. Firing on mere drain
/// spammed every red-alert fight, loudest on the player's own ship, for a
/// condition that never reached the lock the ladder exists to avoid.
///
/// Driven by [`PowerBrownoutState::locked_changed`] — the lock-changed edge
/// [`tick_power_system`] forwards from [`PowerSystem::tick`]. The edge fires on
/// BOTH lock and unlock; only the INTO-locked direction (`power.0.locked()`) is
/// a brownout. On lock every group's owning station is told (helm → Helm,
/// weapons → Tactical, shields → Shields) and the announced set is recorded in
/// `notified_groups` for the intent narration to read while the lock persists;
/// on recovery the set is cleared so a later exhaustion re-announces.
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
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, power, mut brownout_state, control_sources, ship_config) in ships.iter_mut() {
        // Consume this tick's lock-changed edge. Only a transition INTO the
        // locked state is a brownout; an unlock (recovery) consumes the edge and
        // clears the announced set so the next exhaustion re-announces.
        if !std::mem::take(&mut brownout_state.locked_changed) {
            continue;
        }
        if !power.0.locked() {
            brownout_state.notified_groups.clear();
            continue;
        }

        let sender_origin = control_sources
            .0
            .source_for(&crate::ship::system_registry::power_reactor_system_id());

        brownout_state.notified_groups.clear();
        for (group_id, level) in power.0.iter() {
            brownout_state.notified_groups.insert(group_id.0.clone());
            if let Some(address) = address_for_power_group(&ship_config.0, &group_id.0) {
                writer.write(CoordinationEnqueue {
                    source_entity: entity,
                    sender_origin,
                    address,
                    payload: CoordinationPayload::PowerBrownout {
                        group: group_id.0.clone(),
                        label: power_group_label(&group_id.0).to_string(),
                        allocated_level: level,
                    },
                    presentation: crate::core::messages::CoordinationPresentation::new(
                        "coordination.power_brownout.title",
                        "coordination.power_brownout.body",
                    )
                    .with_title_param("label", power_group_label(&group_id.0))
                    .with_body_param("level", level),
                    sender_label: crate::ship::coordination::CHATTER_SENDER_POWER.to_string(),
                    sender_system: crate::ship::system_registry::power_reactor_system_id(),
                });
            }
        }
    }
}

// ── Scripted restraint (issue #1398) ─────────────────────────────────────────

/// The world flag mirroring the hull the crew fly: `weapons_cold.own_ship`.
///
/// Keyed by ROLE rather than by an authored reference name, because the
/// player's hull is not required to declare one — `falling_skyway.toml` gives
/// its `player-ship` an `id` and no `name`, so there is no name to key on and a
/// scenario still has to be able to ask the question.
///
/// Replaces `weapons_hold.own_ship` (issue #1041) key for key: what moved is the
/// state behind it, from a hidden captain's boolean to the ship's own reactor.
pub const OWN_SHIP_WEAPONS_COLD_FLAG: &str = "weapons_cold.own_ship";

/// The world flag mirroring a named ship's cold weapons group:
/// `weapons_cold.<name>`.
///
/// The naming imitates issue #1035's `workforce.<id>.on_strike` deliberately:
/// authoritative state lives in the ship's own [`ShipPowerSystem`], the flag is
/// a MIRROR of it, and scenario script reads the mirror. `name` is the entity's
/// authored reference name — the same string `on_destroyed(...)` and `hail(...)`
/// take.
pub fn weapons_cold_flag(name: &str) -> String {
    format!("weapons_cold.{name}")
}

/// Drain the scenario's queued power orders onto their ships (issue #1398).
///
/// The scripted half of the reactor, and it writes the SAME state an
/// Engineering officer's console command does: `hold_fire(name)` in a world
/// script and `SetPowerGroupAllocation { group: "weapons", level: 0 }` from a
/// Power console both land in
/// [`PowerSystem::set_group_allocation`](crate::modifiers::power_system::PowerSystem::set_group_allocation),
/// so the fire gate has one thing to read and [`mirror_weapons_cold_flags`]
/// below has one thing to publish.
///
/// It writes the reactor directly rather than manufacturing an admitted
/// command, which is the shape every other scripted world effect already has
/// (`destroy_entity`, `damage_infrastructure`, `set_workforce_disposition`).
/// Admission is the boundary between an OPERATOR and the ship — human or AI, the
/// same table either way — and the world is not an operator. What it is not is a
/// bypass: the order goes through the very setter a console order does, so the
/// group's authored `min_level` clamps it exactly the same way. A hull whose
/// `[power_groups.weapons]` says `min_level = 1` cannot be silenced by a script
/// any more than by its own crew, and that refusal is a designer's decision in
/// the entity TOML rather than this system's.
///
/// [`ScriptedPowerLevel::AuthoredDefault`] is resolved HERE and nowhere else,
/// because here is the first place that holds both the ship and its
/// [`crate::ship_plugin::ShipConfigComponent`]: the Rhai closure that pushed the
/// order has no world access at all, and a script that wrote `2` would be
/// authoring one hull's number into every scenario that used the verb.
pub fn drain_scripted_power_orders(
    // The scripted power-order queue, owned and declared by [`ShipPowerPlugin`].
    // `Option` so a reduced test app that runs this system without the
    // registering plugin is a no-op rather than a panic.
    power_orders: Option<ResMut<crate::effect_queue::EffectQueue<PendingGroupPower>>>,
    mut ships: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &mut ShipPowerSystem,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let Some(mut power_orders) = power_orders else {
        return;
    };
    // A `Deref` read, so a world that queues nothing never marks the queue
    // changed — the `tick_operations` precedent.
    if power_orders.0.is_empty() {
        return;
    }
    let queued = std::mem::take(&mut power_orders.0);
    for order in queued {
        let mut found = false;
        for (ship_uuid, mut power, ship_config) in ships.iter_mut() {
            if ship_uuid.0 != order.uuid {
                continue;
            }
            found = true;
            let level = match order.level {
                ScriptedPowerLevel::Exact(level) => level,
                ScriptedPowerLevel::AuthoredDefault => ship_config
                    .and_then(|config| config.0.power_groups.get(&order.group))
                    .map(|group| group.default_level)
                    .unwrap_or_else(crate::ship::config::default_power_level),
            };
            if let Err(err) = power.0.set_group_allocation(&order.group, level) {
                // A machine token, not prose: `src/ship/power.rs` is on
                // `check-strings.mjs`'s wire-visible allowlist, where every
                // prose literal in the production region is an error — spaces
                // and capitals included, so the uuid rides a colon.
                warn!("power.scripted_order_ignored:{}:{err:?}", order.uuid);
            }
        }
        if !found {
            warn!("power.scripted_order_unknown_ship:{}", order.uuid);
        }
    }
}

/// Mirror every ship's `weapons` power group into the world flag store (issue
/// #1398), so scenario script can read the posture and react to it.
///
/// Replaces `console::captain::server::mirror_weapons_hold_flags` (issue #1041)
/// key for key: `weapons_cold.own_ship` for the hull the crew fly, and
/// `weapons_cold.<name>` for any ship carrying an authored reference name. The
/// role key exists because a world's player hull is not required to declare a
/// name (`falling_skyway.toml` gives its `player-ship` an `id` and no `name`),
/// and "have the crew gone cold?" is exactly the question a scenario wants to
/// ask.
///
/// The transition is decided from the store's own `(before, after)` and the
/// event pushed onto `pending_world_events`, exactly as
/// `infrastructure::server::mirror_flags` does — so an
/// `on_flag_set("weapons_cold.own_ship", …)` handler chains off an order on the
/// next pipeline pass, through machinery that was already there.
///
/// # Why every ship every tick, and why that is still cheap
///
/// The retired hold mirror could filter on `Changed<ShipWeaponsHold>` because
/// that component only moved when somebody pulled the lever. [`ShipPowerSystem`]
/// is not like that: `tick_power_system` takes it by `&mut` to integrate the
/// battery, so change detection fires for every ship on every tick and the
/// filter would buy nothing but a false sense of one. So the walk is
/// unconditional — and the store is READ before it is written, which is the part
/// that actually matters. Reading first is not an optimisation: it is what keeps
/// a world nobody pulls the lever in byte-identical, because a `DerefMut` on
/// `WorldContentRuntime` marks the resource changed and change detection on it
/// is read elsewhere (`probe_radiation.toml` is where that showed up under
/// #1041, with the lever untouched and its committed digest moved).
///
/// Rows are sorted by flag name before anything is written, so the order of the
/// emitted events is a function of the content and never of archetype iteration
/// order.
pub fn mirror_weapons_cold_flags(
    runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    ships: Query<
        (
            &ShipPowerSystem,
            Option<&crate::entities::spawner::EntityName>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    if ships.is_empty() {
        return;
    }
    let weapons = PowerGroupId(WEAPONS_POWER_GROUP.to_string());
    let mut writes: Vec<(String, bool)> = Vec::new();
    for (power, name, is_local) in ships.iter() {
        let cold = power.0.is_group_cold(&weapons);
        if is_local {
            writes.push((OWN_SHIP_WEAPONS_COLD_FLAG.to_string(), cold));
        }
        if let Some(name) = name {
            writes.push((weapons_cold_flag(&name.0), cold));
        }
    }
    writes.sort();
    let pending: Vec<(String, bool)> = writes
        .into_iter()
        .filter(|(flag, cold)| runtime.flags.flag(flag) != *cold)
        .collect();
    if pending.is_empty() {
        return;
    }
    for (flag, cold) in pending {
        if cold {
            runtime.flags.set_flag(&flag);
        } else {
            runtime.flags.clear_flag(&flag);
        }
        runtime.pending_world_events.push(if cold {
            crate::world::content::WorldEvent::FlagSet {
                name: flag,
                origin_layer: None,
            }
        } else {
            crate::world::content::WorldEvent::FlagCleared {
                name: flag,
                origin_layer: None,
            }
        });
    }
}

// ── Blackboard publish (issue #561) ──────────────────────────────────────────

pub(crate) fn publish_power_blackboard(
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
    // The authored hull, for each group's `min_level` (issue #1004). Read via
    // its OWN query for the same reason `control_sources_q` below is: the
    // primary `ship_q` requires a per-entity `ShipPowerSystem`, so a LocalShip
    // running off the global resources would silently contribute nothing and
    // every group would publish the fallback floor instead of its authored one.
    ship_config_q: Query<
        &crate::ship_plugin::ShipConfigComponent,
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
    use crate::ship::system_registry::{
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
    let reactor_id = crate::ship::system_registry::power_reactor_system_id();
    let battery_id = crate::ship::system_registry::power_battery_system_id();
    let reactor_online = control_sources
        .map(|cs| !cs.0.is_offline(&reactor_id))
        .unwrap_or(true);
    let battery_online = control_sources
        .map(|cs| !cs.0.is_offline(&battery_id))
        .unwrap_or(true);

    let ship_config = ship_config_q.single().ok();

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
            // This group's own authored floor — the lowest level it may be
            // commanded to (issue #1004, made a real engine clamp by #1395). A
            // hull that declares no `[power_groups.*]` block has none to read,
            // so the parse default stands in, which is also the floor
            // `PowerSystem` seeds. Read off the hull rather than off the
            // multiplier table, since the table's LENGTH is a ceiling and says
            // nothing about where the rungs start.
            //
            // The same number the reactor is holding: `PowerSystem::floor_for`
            // was seeded from this very field at spawn. Published from the
            // config rather than from the reactor because the blackboard is
            // built from the hull throughout, and the two cannot disagree.
            let min_level = ship_config
                .and_then(|sc| sc.0.power_groups.get(&gid))
                .map(|g| g.min_level)
                .unwrap_or_else(crate::ship::config::default_min_power_level);
            PowerGroupEntry {
                id: gid.0.clone(),
                label: power_group_label(gid.0.as_str()).into(),
                level: power_level_for(&power.0, &gid),
                // The standing order, so the panel's +/- can step from what was
                // ASKED for rather than from what a battery floor has left the
                // group running at (issue #952).
                commanded_level: power.0.commanded_level_for(&gid),
                min_level,
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
#[path = "power_tests.rs"]
mod tests;
